//! ControlHub administrator permissions and built-in system roles. The grant rules are the
//! project ones (`crate::permissions`), with the Super Admin in place of the project Owner.

use std::collections::BTreeSet;

use uuid::Uuid;

use crate::permissions::{Actor, PermissionKind, RuleText, Target};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SystemPermission {
    ProjectsView,
    ProjectsManage,
    UsersView,
    UsersManage,
    TeamView,
    TeamManage,
    RolesManage,
    AuditView,
}

pub type SystemPermissionSet = BTreeSet<SystemPermission>;

impl SystemPermission {
    pub const ALL: [SystemPermission; 8] = [
        SystemPermission::ProjectsView,
        SystemPermission::ProjectsManage,
        SystemPermission::UsersView,
        SystemPermission::UsersManage,
        SystemPermission::TeamView,
        SystemPermission::TeamManage,
        SystemPermission::RolesManage,
        SystemPermission::AuditView,
    ];
}

impl PermissionKind for SystemPermission {
    const ALL: &'static [Self] = &SystemPermission::ALL;
    const MEMBERS_MANAGE: Self = SystemPermission::TeamManage;
    const ROLES_MANAGE: Self = SystemPermission::RolesManage;
    const TEXT: RuleText = RuleText {
        own_change: "you cannot change your own administrator access",
        needs_members_manage: "requires the system.team.manage permission",
        owner_changes_owner: "only a Super Admin can change another Super Admin",
        stronger_target: "you cannot change an administrator who has permissions you do not have",
        owner_grants_owner: "only a Super Admin can grant the Super Admin role",
        grant_exceeds_actor: "you can only grant permissions you have yourself",
        last_owner: "ControlHub needs at least one Super Admin",
        owner_invites_owner: "only a Super Admin can grant the Super Admin role",
        invite_exceeds_actor: "you can only grant permissions you have yourself",
        needs_roles_manage: "requires the system.roles.manage permission",
        stronger_template: "you cannot change a role that has permissions you do not have",
        template_exceeds_actor: "a role can only contain permissions you have yourself",
    };

    fn id(self) -> &'static str {
        match self {
            SystemPermission::ProjectsView => "system.projects.view",
            SystemPermission::ProjectsManage => "system.projects.manage",
            SystemPermission::UsersView => "system.users.view",
            SystemPermission::UsersManage => "system.users.manage",
            SystemPermission::TeamView => "system.team.view",
            SystemPermission::TeamManage => "system.team.manage",
            SystemPermission::RolesManage => "system.roles.manage",
            SystemPermission::AuditView => "system.audit.view",
        }
    }

    fn title(self) -> &'static str {
        match self {
            SystemPermission::ProjectsView => "View all projects",
            SystemPermission::ProjectsManage => {
                "Create, rename and delete projects; manage their environments and members"
            }
            SystemPermission::UsersView => "View all accounts",
            SystemPermission::UsersManage => "Disable and enable accounts, end their sessions",
            SystemPermission::TeamView => "View administrators",
            SystemPermission::TeamManage => "Grant and change administrator access",
            SystemPermission::RolesManage => "Manage administrator role templates",
            SystemPermission::AuditView => "View the audit log of every project",
        }
    }

    fn section(self) -> &'static str {
        match self {
            SystemPermission::ProjectsView | SystemPermission::ProjectsManage => "Projects",
            SystemPermission::UsersView | SystemPermission::UsersManage => "Accounts",
            SystemPermission::TeamView | SystemPermission::TeamManage | SystemPermission::RolesManage => {
                "Administrators"
            }
            SystemPermission::AuditView => "Audit",
        }
    }

    fn requires(self) -> Option<Self> {
        match self {
            SystemPermission::ProjectsManage => Some(SystemPermission::ProjectsView),
            SystemPermission::UsersManage => Some(SystemPermission::UsersView),
            SystemPermission::TeamManage | SystemPermission::RolesManage => Some(SystemPermission::TeamView),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemBuiltin {
    SuperAdmin,
    Admin,
    Auditor,
}

impl SystemBuiltin {
    pub const ALL: [SystemBuiltin; 3] = [SystemBuiltin::SuperAdmin, SystemBuiltin::Admin, SystemBuiltin::Auditor];

    pub fn key(self) -> &'static str {
        match self {
            SystemBuiltin::SuperAdmin => "super_admin",
            SystemBuiltin::Admin => "admin",
            SystemBuiltin::Auditor => "auditor",
        }
    }

    #[cfg(test)]
    pub fn name(self) -> &'static str {
        match self {
            SystemBuiltin::SuperAdmin => "Super Admin",
            SystemBuiltin::Admin => "Administrator",
            SystemBuiltin::Auditor => "Auditor",
        }
    }

    pub fn permissions(self) -> SystemPermissionSet {
        use SystemPermission::*;
        match self {
            SystemBuiltin::SuperAdmin | SystemBuiltin::Admin => SystemPermission::ALL.into_iter().collect(),
            SystemBuiltin::Auditor => [ProjectsView, UsersView, TeamView, AuditView].into(),
        }
    }
}

/// May `actor` disable, enable or sign out the account `target_id`? `target_admin` is the
/// account's administrator grant, if it has one. The last active Super Admin is protected in
/// storage as well.
pub fn check_account_change(
    actor: &Actor<SystemPermission>,
    target_id: Uuid,
    target_admin: Option<&Target<SystemPermission>>,
) -> Result<(), &'static str> {
    if actor.user_id == target_id {
        return Err("you cannot change your own account here; use your account settings");
    }
    if !actor.has(SystemPermission::UsersManage) {
        return Err("requires the system.users.manage permission");
    }
    if let Some(t) = target_admin {
        if t.is_owner && !actor.is_owner {
            return Err("only a Super Admin can change a Super Admin's account");
        }
        if !actor.is_owner && !t.permissions.is_subset(&actor.permissions) {
            return Err("you cannot change an administrator who has permissions you do not have");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::permissions::{
        Grant, catalog_of, check_member_change, check_template_change, from_stored_of, normalise, parse_set_of,
    };
    use SystemPermission::*;
    use proptest::prelude::*;

    fn set(ps: &[SystemPermission]) -> SystemPermissionSet {
        ps.iter().copied().collect()
    }

    fn actor(ps: &[SystemPermission], super_admin: bool) -> Actor<SystemPermission> {
        Actor { user_id: Uuid::from_u128(1), permissions: normalise(set(ps)), is_owner: super_admin }
    }

    fn target(perms: SystemPermissionSet, super_admin: bool) -> Target<SystemPermission> {
        Target { user_id: Uuid::from_u128(2), is_owner: super_admin, permissions: perms }
    }

    #[test]
    fn ids_are_unique_exact_and_never_collide_with_project_permissions() {
        for p in SystemPermission::ALL {
            assert_eq!(SystemPermission::from_id(p.id()), Some(p));
            assert!(p.id().starts_with("system."), "{}", p.id());
            assert!(crate::permissions::Permission::from_id(p.id()).is_none());
        }
        for project in crate::permissions::Permission::ALL {
            assert_eq!(SystemPermission::from_id(project.as_str()), None, "project ids grant nothing system-wide");
        }
        for bad in ["", "system.*", "System.projects.view", " system.audit.view", "super_admin"] {
            assert_eq!(SystemPermission::from_id(bad), None, "{bad:?}");
        }
        let ids: BTreeSet<&str> = SystemPermission::ALL.iter().map(|p| p.id()).collect();
        assert_eq!(ids.len(), SystemPermission::ALL.len());
    }

    #[test]
    fn client_input_is_strict_and_stored_values_are_lenient() {
        assert_eq!(
            parse_set_of::<SystemPermission>(&["system.users.manage".into()]).unwrap(),
            set(&[UsersManage, UsersView])
        );
        assert_eq!(parse_set_of::<SystemPermission>(&["telemetry.view".into()]), Err("unknown permission"));
        assert_eq!(
            from_stored_of::<SystemPermission>(&["system.audit.view".into(), "system.future".into()]),
            set(&[AuditView])
        );
    }

    #[test]
    fn managing_implies_viewing() {
        assert_eq!(normalise(set(&[ProjectsManage])), set(&[ProjectsManage, ProjectsView]));
        assert_eq!(normalise(set(&[RolesManage])), set(&[RolesManage, TeamView]));
        assert_eq!(normalise(set(&[AuditView])), set(&[AuditView]));
    }

    #[test]
    fn builtin_roles_match_the_migration() {
        let sql = include_str!("../../../infra/migrations/postgres/0006_system_roles.sql");
        for b in SystemBuiltin::ALL {
            let start = sql.find(&format!("('{}', ARRAY[", b.name())).unwrap_or_else(|| panic!("{} missing", b.name()));
            let rest = &sql[start..];
            let body = &rest[rest.find('[').unwrap() + 1..rest.find(']').unwrap()];
            let ids: Vec<String> = body
                .split(',')
                .map(|s| s.trim().trim_matches(|c: char| c == '\'' || c.is_whitespace()).to_owned())
                .collect();
            assert_eq!(parse_set_of::<SystemPermission>(&ids).unwrap(), b.permissions(), "{}", b.name());
            assert!(rest[..rest.find(')').unwrap()].contains(&format!("'{}'", b.key())), "{} key", b.name());
            assert_eq!(normalise(b.permissions()), b.permissions(), "closed under implication");
        }
        assert_eq!(SystemBuiltin::SuperAdmin.key(), controlhub_storage::admin::SUPER_ADMIN);
    }

    #[test]
    fn only_super_admins_touch_super_admins_and_one_always_remains() {
        let admin = actor(&SystemPermission::ALL, false);
        let super_role = Grant::Role { permissions: SystemBuiltin::SuperAdmin.permissions(), owner: true };
        let sa = target(SystemBuiltin::SuperAdmin.permissions(), true);
        assert_eq!(
            check_member_change(&admin, &target(set(&[]), false), &super_role, 1),
            Err("only a Super Admin can grant the Super Admin role")
        );
        assert_eq!(
            check_member_change(&admin, &sa, &Grant::Remove, 2),
            Err("only a Super Admin can change another Super Admin")
        );
        let root = actor(&SystemPermission::ALL, true);
        assert_eq!(check_member_change(&root, &sa, &Grant::Remove, 2), Ok(()));
        assert_eq!(
            check_member_change(&root, &sa, &Grant::Remove, 1),
            Err("ControlHub needs at least one Super Admin")
        );
        let me = Target { user_id: root.user_id, is_owner: true, permissions: SystemBuiltin::SuperAdmin.permissions() };
        assert_eq!(
            check_member_change(&root, &me, &Grant::Remove, 5),
            Err("you cannot change your own administrator access")
        );
    }

    #[test]
    fn an_auditor_changes_nothing() {
        let auditor = actor(&SystemBuiltin::Auditor.permissions().into_iter().collect::<Vec<_>>(), false);
        assert_eq!(
            check_member_change(&auditor, &target(set(&[]), false), &Grant::Individual(set(&[AuditView])), 1),
            Err("requires the system.team.manage permission")
        );
        assert_eq!(
            check_template_change(&auditor, None, &set(&[])),
            Err("requires the system.roles.manage permission")
        );
    }

    #[test]
    fn account_changes_follow_the_same_reach_as_grants() {
        let plain = Uuid::from_u128(9);
        let manager = actor(&[UsersManage], false);
        assert_eq!(check_account_change(&manager, plain, None), Ok(()), "plain accounts");
        assert_eq!(
            check_account_change(&manager, manager.user_id, None),
            Err("you cannot change your own account here; use your account settings")
        );
        let auditor = target(SystemBuiltin::Auditor.permissions(), false);
        assert_eq!(
            check_account_change(&manager, auditor.user_id, Some(&auditor)),
            Err("you cannot change an administrator who has permissions you do not have")
        );
        let admin = actor(&SystemPermission::ALL, false);
        assert_eq!(check_account_change(&admin, auditor.user_id, Some(&auditor)), Ok(()));
        let sa = target(SystemBuiltin::SuperAdmin.permissions(), true);
        assert_eq!(
            check_account_change(&admin, sa.user_id, Some(&sa)),
            Err("only a Super Admin can change a Super Admin's account")
        );
        assert_eq!(check_account_change(&actor(&SystemPermission::ALL, true), sa.user_id, Some(&sa)), Ok(()));
        assert_eq!(
            check_account_change(&actor(&[UsersView], false), plain, None),
            Err("requires the system.users.manage permission")
        );
    }

    #[test]
    fn catalog_lists_every_permission_once() {
        let c = catalog_of::<SystemPermission>();
        assert_eq!(c.len(), SystemPermission::ALL.len());
        assert!(c.iter().all(|e| !e.label.is_empty() && !e.group.is_empty()));
    }

    fn arb_set() -> impl Strategy<Value = SystemPermissionSet> {
        proptest::collection::btree_set(
            proptest::sample::select(SystemPermission::ALL.to_vec()),
            0..=SystemPermission::ALL.len(),
        )
    }

    proptest! {
        /// Random administrator changes by random administrators never hand out a permission the
        /// actor lacked, never let a non-Super Admin touch or create a Super Admin, and always
        /// leave a Super Admin.
        #[test]
        fn no_sequence_of_admin_changes_escalates(ops in proptest::collection::vec((0usize..4, 0usize..4, arb_set(), any::<bool>(), any::<bool>()), 0..60)) {
            let mut admins: Vec<Option<(SystemPermissionSet, bool)>> = vec![
                Some((SystemBuiltin::SuperAdmin.permissions(), true)),
                Some((SystemBuiltin::Admin.permissions(), false)),
                Some((SystemBuiltin::Auditor.permissions(), false)),
                Some((SystemPermissionSet::new(), false)),
            ];
            for (ai, ti, perms, make_super, remove) in ops {
                let (Some((a_perms, a_super)), Some((t_perms, t_super))) = (admins[ai].clone(), admins[ti].clone()) else { continue };
                let actor = Actor { user_id: Uuid::from_u128(ai as u128), permissions: a_perms.clone(), is_owner: a_super };
                let t = Target { user_id: Uuid::from_u128(ti as u128), is_owner: t_super, permissions: t_perms.clone() };
                let supers = admins.iter().flatten().filter(|m| m.1).count() as i64;
                let grant = if remove {
                    Grant::Remove
                } else if make_super {
                    Grant::Role { permissions: SystemBuiltin::SuperAdmin.permissions(), owner: true }
                } else {
                    Grant::Individual(normalise(perms))
                };
                if check_member_change(&actor, &t, &grant, supers).is_err() { continue; }
                prop_assert!(a_super || !t_super, "only a Super Admin changes a Super Admin");
                admins[ti] = match grant {
                    Grant::Individual(p) => { prop_assert!(p.is_subset(&a_perms)); Some((p, false)) }
                    Grant::Role { permissions, owner } => { prop_assert!(!owner || a_super); Some((permissions, owner)) }
                    Grant::Remove => None,
                };
                prop_assert!(admins.iter().flatten().any(|m| m.1), "a Super Admin always remains");
            }
        }
    }
}
