-- Incident history belongs to a scope, not to the lifetime of the operator account.
-- Removing a project/environment removes its queue; removing a user keeps the
-- incident and its timestamped acknowledgment.

ALTER TABLE incident_rules ALTER COLUMN created_by DROP NOT NULL;
ALTER TABLE incident_rules DROP CONSTRAINT incident_rules_created_by_fkey;
ALTER TABLE incident_rules ADD CONSTRAINT incident_rules_created_by_fkey
    FOREIGN KEY (created_by) REFERENCES users(id) ON DELETE SET NULL;
ALTER TABLE incident_rules DROP CONSTRAINT incident_rules_tenant_id_fkey;
ALTER TABLE incident_rules ADD CONSTRAINT incident_rules_tenant_id_fkey
    FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE CASCADE;
ALTER TABLE incident_rules DROP CONSTRAINT incident_rules_project_id_fkey;
ALTER TABLE incident_rules ADD CONSTRAINT incident_rules_project_id_fkey
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE;
ALTER TABLE incident_rules DROP CONSTRAINT incident_rules_environment_id_fkey;
ALTER TABLE incident_rules ADD CONSTRAINT incident_rules_environment_id_fkey
    FOREIGN KEY (environment_id) REFERENCES environments(id) ON DELETE CASCADE;

ALTER TABLE incidents DROP CONSTRAINT incidents_rule_id_fkey;
ALTER TABLE incidents ADD CONSTRAINT incidents_rule_id_fkey
    FOREIGN KEY (rule_id) REFERENCES incident_rules(id) ON DELETE CASCADE;
ALTER TABLE incidents DROP CONSTRAINT incidents_tenant_id_fkey;
ALTER TABLE incidents ADD CONSTRAINT incidents_tenant_id_fkey
    FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE CASCADE;
ALTER TABLE incidents DROP CONSTRAINT incidents_project_id_fkey;
ALTER TABLE incidents ADD CONSTRAINT incidents_project_id_fkey
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE;
ALTER TABLE incidents DROP CONSTRAINT incidents_environment_id_fkey;
ALTER TABLE incidents ADD CONSTRAINT incidents_environment_id_fkey
    FOREIGN KEY (environment_id) REFERENCES environments(id) ON DELETE CASCADE;
ALTER TABLE incidents DROP CONSTRAINT incidents_acknowledged_by_fkey;
ALTER TABLE incidents ADD CONSTRAINT incidents_acknowledged_by_fkey
    FOREIGN KEY (acknowledged_by) REFERENCES users(id) ON DELETE SET NULL;
