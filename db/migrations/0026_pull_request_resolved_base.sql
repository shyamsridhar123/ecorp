ALTER TABLE pull_request_publications
    ADD COLUMN IF NOT EXISTS pull_request_base_ref TEXT;

ALTER TABLE pull_request_publications
    ADD CONSTRAINT pull_request_publications_resolved_base_check
    CHECK (
        pull_request_number IS NULL
        OR (
            pull_request_base_ref IS NOT NULL
            AND length(pull_request_base_ref) BETWEEN 1 AND 512
        )
    );
