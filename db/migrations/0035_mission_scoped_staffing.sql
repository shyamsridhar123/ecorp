-- Keep identities for historical attribution; retirement never deletes work.
ALTER TABLE agents
    ADD COLUMN mission_id UUID REFERENCES missions(id) ON DELETE RESTRICT,
    ADD COLUMN pinned BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN retired_at TIMESTAMPTZ;

CREATE INDEX agents_mission_staffing_idx ON agents(corp_id, mission_id)
    WHERE mission_id IS NOT NULL;

CREATE INDEX agents_available_roster_idx ON agents(corp_id, pinned)
    WHERE retired_at IS NULL;
