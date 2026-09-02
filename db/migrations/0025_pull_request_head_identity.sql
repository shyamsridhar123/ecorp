ALTER TABLE pull_request_publications
    ADD COLUMN IF NOT EXISTS pull_request_head_sha TEXT,
    ADD COLUMN IF NOT EXISTS pull_request_head_repository_owner TEXT,
    ADD COLUMN IF NOT EXISTS pull_request_is_cross_repository BOOLEAN;

ALTER TABLE pull_request_publications
    ADD CONSTRAINT pull_request_publications_verified_head_check
    CHECK (
        pull_request_number IS NULL
        OR (
            pull_request_head_sha ~ '^[0-9a-f]{40,64}$'
            AND pull_request_head_repository_owner IS NOT NULL
            AND length(pull_request_head_repository_owner) BETWEEN 1 AND 100
            AND pull_request_is_cross_repository = FALSE
        )
    );
