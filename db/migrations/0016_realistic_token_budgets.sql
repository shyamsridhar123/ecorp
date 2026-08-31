ALTER TABLE corp_budget_policies
    ALTER COLUMN actor_tokens_per_24h SET DEFAULT 4000000,
    ALTER COLUMN corp_tokens_per_24h SET DEFAULT 20000000;

UPDATE corp_budget_policies
SET actor_tokens_per_24h = 4000000,
    corp_tokens_per_24h = 20000000,
    updated_at = now()
WHERE actor_tokens_per_24h = 500000
  AND corp_tokens_per_24h = 5000000;
