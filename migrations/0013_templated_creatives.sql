-- Server-side template rendering for `templated` creatives.
--
-- Refs:
--   API.md § 1 (decision response oneOf adds `templated`),
--   API.md § 3.5 (Creatives — `templated` request body shape),
--   API.md § 3.6 (CreativeTemplate `template` + `templateEngine`),
--   PHASES.md task 4.8.
--
-- Two changes:
--
-- (1) creative_templates carries an optional `template` field
--     holding the renderer source (today: Liquid). When NULL, the
--     template is input-validation-only — only `native` creatives
--     can reference it. When set, `templated` creatives can
--     reference it for decision-time SSR. `template_engine` names
--     the engine (today: 'liquid'); future engines land additively
--     without a breaking schema change.
--
-- (2) creatives.kind grows a fourth value, `'templated'`. The
--     check-constraint update is a DROP + re-ADD because Postgres
--     can't ALTER CHECK constraint expressions in place. The
--     `_v2` suffix on the new constraint is convention so future
--     migrations can spot the v2 generation.
--
-- Both changes are purely additive at the data layer. No backfill
-- needed: existing rows have NULL template + image/html/native
-- kinds, all of which remain valid.
--
-- RLS: creative_templates already has the
-- `creative_templates_tenant` policy (project-bound). No new
-- policy needed.

SET search_path TO knievel, public;

ALTER TABLE knievel.creative_templates
    ADD COLUMN IF NOT EXISTS template        text,
    ADD COLUMN IF NOT EXISTS template_engine text;

-- Either both NULL (input-validation only) or both NOT NULL with
-- engine = 'liquid' (the only engine v0 ships). A bad combination
-- (template set, engine NULL — or engine non-NULL when template
-- is NULL) is a write-time bug; surface it loudly.
ALTER TABLE knievel.creative_templates
    DROP CONSTRAINT IF EXISTS creative_templates_template_engine_pair;
ALTER TABLE knievel.creative_templates
    ADD  CONSTRAINT creative_templates_template_engine_pair CHECK (
        (template IS NULL AND template_engine IS NULL)
        OR (template IS NOT NULL AND template_engine = 'liquid')
    );

ALTER TABLE knievel.creatives
    DROP CONSTRAINT IF EXISTS creatives_kind_check;
ALTER TABLE knievel.creatives
    ADD  CONSTRAINT creatives_kind_check_v2 CHECK (
        kind IN ('image', 'html', 'native', 'templated')
    );
