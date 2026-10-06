-- Actions are run by whoever holds the approve permission (ADR-0012, amended). A plan proposed
-- by such a user is approved at once, so the approver may now be the proposer.

DO $$
DECLARE
    c text;
BEGIN
    FOR c IN
        SELECT conname FROM pg_constraint
        WHERE conrelid = 'action_plans'::regclass AND contype = 'c'
          AND pg_get_constraintdef(oid) LIKE '%decided_by <> created_by%'
    LOOP
        EXECUTE format('ALTER TABLE action_plans DROP CONSTRAINT %I', c);
    END LOOP;
END $$;
