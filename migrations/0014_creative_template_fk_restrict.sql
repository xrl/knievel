-- Strengthen the FK from `creatives.template_id` to
-- `creative_templates.id` with ON DELETE RESTRICT.
--
-- The original declaration in 0006_demand.sql had no ON DELETE clause,
-- which defaults to NO ACTION (deferred RI check at statement end).
-- That means an attempt to DELETE a creative_template that still has
-- referencing creatives rows fails at commit time with a FK violation
-- error — but the error surface is less clear than the explicit RESTRICT
-- wording, and it interacts poorly with any future deferred-constraint
-- session tweaks.
--
-- ON DELETE RESTRICT raises the error immediately at DELETE time,
-- making it clear why the operation was rejected and which rows block
-- it. The semantics match our intent: a template that live `templated`
-- or `native` creatives reference must not be deleted until those
-- creatives are removed or reassigned.
--
-- Refs:
--   opus audit O18 — `creatives.template_id` FK missing ON DELETE clause.
--   API.md § 3.5 (`templated` creative references a CreativeTemplate).
--   API.md § 3.6 (CreativeTemplate lifecycle).

SET search_path TO knievel, public;

-- Postgres auto-names the FK `creatives_template_id_fkey` when no
-- explicit name is given in the REFERENCES clause. Drop it and re-add
-- with the explicit ON DELETE RESTRICT action.
ALTER TABLE knievel.creatives
    DROP CONSTRAINT IF EXISTS creatives_template_id_fkey;

ALTER TABLE knievel.creatives
    ADD CONSTRAINT creatives_template_id_fkey
        FOREIGN KEY (template_id)
        REFERENCES knievel.creative_templates(id)
        ON DELETE RESTRICT;
