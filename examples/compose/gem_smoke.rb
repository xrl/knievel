#!/usr/bin/env ruby
# frozen_string_literal: true

# End-to-end gem smoke for `examples/compose/`. Requires the
# `knievel` gem to be already installed (`gem install
# knievel-X.Y.Z.gem`) — the gem isn't on RubyGems for the
# in-CI flow that builds-then-smokes the not-yet-published
# version. Manual local invocation:
#
#   docker compose -f examples/compose/compose.yaml up -d
#   gem install pkg/knievel-X.Y.Z.gem
#   KNIEVEL_TOKEN="$(cat tmp/knievel-dev-token)" \
#     ruby examples/compose/gem_smoke.rb
#   docker compose -f examples/compose/compose.yaml down
#
# Project id is derived from the well-known external ids the
# `knievel-cli seed-demo` sidecar uses (`demo-org`,
# `demo-project`) — same `sha256(...)[:12]` shape `src/orgs.rs`
# and `src/cli/seed_demo.rs` apply server-side.

require "digest"
require "knievel/client"

HOST = ENV.fetch("KNIEVEL_HOST", "http://localhost:8080")
TOKEN = ENV.fetch("KNIEVEL_TOKEN")
ORG_EXTERNAL_ID = ENV.fetch("KNIEVEL_ORG_EXTERNAL_ID", "demo-org")
PROJECT_EXTERNAL_ID = ENV.fetch("KNIEVEL_PROJECT_EXTERNAL_ID", "demo-project")

def derive_org_id(external_id)
  "org_#{Digest::SHA256.hexdigest(external_id)[0, 12]}"
end

def derive_project_id(org_id, external_id)
  digest = Digest::SHA256.hexdigest("#{org_id}/#{external_id}")
  "pj_#{digest[0, 12]}"
end

org_id = derive_org_id(ORG_EXTERNAL_ID)
project_id = derive_project_id(org_id, PROJECT_EXTERNAL_ID)

puts "host:        #{HOST}"
puts "org_id:      #{org_id}"
puts "project_id:  #{project_id}"

client = Knievel::Client.new(host: HOST, access_token: TOKEN)

# 1. Walk every advertiser via the Enumerable wrapper. seed-demo
#    upserts one (`demo-advertiser`) so we expect at least one
#    row. The walk also exercises the cursor-walk plumbing even
#    though one row fits in the first (and only) page.
print "  walk advertisers ... "
advertisers = client.advertisers(project_id).to_a
puts "#{advertisers.size} row(s)"
abort "expected at least 1 advertiser from seed-demo" if advertisers.empty?

# 2. each_page yields a per-page array. With page_size: 1 against
#    a single seeded row, we get exactly one page (no nextCursor
#    even with the `+1` peek because there's nothing to peek).
print "  each_page (page_size=1) ... "
pages = []
client.advertisers(project_id, page_size: 1).each_page { |p| pages << p.size }
puts "#{pages.size} page(s) sized #{pages.inspect}"
abort "each_page should yield at least one page" if pages.empty?

# 3. lazy.first(n) short-circuits — proves Enumerable laziness
#    works against the live HTTP wrapper, not just rspec doubles.
print "  lazy.first(1) ... "
got = client.advertisers(project_id, page_size: 1).lazy.first(1)
puts "#{got.size} record"
abort "lazy.first(1) should return one record" unless got.size == 1

# 4. Same shape on a second resource — guards against the
#    base-class abstraction silently working only for
#    Advertisers.
print "  walk campaigns ... "
campaigns = client.campaigns(project_id).to_a
puts "#{campaigns.size} row(s)"
abort "expected at least 1 campaign from seed-demo" if campaigns.empty?

puts "GEM SMOKE OK"
