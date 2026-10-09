-- PostgreSQL notifications are transactional hints used by API replicas to fan out action plan
-- changes to authorized WebSocket subscribers. The payload contains scope and row ID only.
CREATE OR REPLACE FUNCTION notify_action_plan_change() RETURNS trigger AS $$
DECLARE
    changed action_plans%ROWTYPE;
BEGIN
    IF TG_OP = 'DELETE' THEN
        changed := OLD;
    ELSE
        changed := NEW;
    END IF;
    PERFORM pg_notify(
        'karbird_action_plan_changed',
        json_build_object(
            'tenant_id', changed.tenant_id,
            'project_id', changed.project_id,
            'environment_id', changed.environment_id,
            'plan_id', changed.id
        )::text
    );
    RETURN changed;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS action_plan_change_notification ON action_plans;
CREATE TRIGGER action_plan_change_notification
AFTER INSERT OR UPDATE OR DELETE ON action_plans
FOR EACH ROW EXECUTE FUNCTION notify_action_plan_change();
