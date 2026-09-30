//! Project permissions, built-in roles and the rules that keep anyone from granting more than
//! they have. Pure (no I/O): handlers load the actor and target, these functions decide.

use std::collections::BTreeSet;

use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Permission {
    TelemetryView,
    TokensView,
    TokensManage,
    ActionsView,
    ActionsPropose,
    ActionsApprove,
    MembersView,
    MembersManage,
    RolesManage,
    ProjectManage,
}

pub type PermissionSet = BTreeSet<Permission>;

impl Permission {
    pub const ALL: [Permission; 10] = [
        Permission::TelemetryView,
        Permission::TokensView,
        Permission::TokensManage,
        Permission::ActionsView,
        Permission::ActionsPropose,
        Permission::ActionsApprove,
        Permission::MembersView,
        Permission::MembersManage,
        Permission::RolesManage,
        Permission::ProjectManage,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Permission::TelemetryView => "telemetry.view",
            Permission::TokensView => "tokens.view",
            Permission::TokensManage => "tokens.manage",
            Permission::ActionsView => "actions.view",
            Permission::ActionsPropose => "actions.propose",
            Permission::ActionsApprove => "actions.approve",
            Permission::MembersView => "members.view",
            Permission::MembersManage => "members.manage",
            Permission::RolesManage => "roles.manage",
            Permission::ProjectManage => "project.manage",
        }
    }

    /// Exact match only: no trimming, no case folding, no wildcards.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            Permission::TelemetryView => "View map, timeline, traces and logs",
            Permission::TokensView => "View tokens",
            Permission::TokensManage => "Create, rotate and revoke tokens",
            Permission::ActionsView => "View action plans",
            Permission::ActionsPropose => "Propose actions",
            Permission::ActionsApprove => "Approve and reject actions",
            Permission::MembersView => "View members",
            Permission::MembersManage => "Manage members and their roles",
            Permission::RolesManage => "Manage role templates",
            Permission::ProjectManage => "Manage project settings",
        }
    }

    pub fn group(self) -> &'static str {
        match self {
            Permission::TelemetryView => "Telemetry",
            Permission::TokensView | Permission::TokensManage => "Tokens",
            Permission::ActionsView | Permission::ActionsPropose | Permission::ActionsApprove => "Actions",
            Permission::MembersView | Permission::MembersManage | Permission::RolesManage => "Team",
            Permission::ProjectManage => "Project",
        }
    }

    /// Permissions this one cannot work without (managing tokens means seeing them, …).
    fn implies(self) -> Option<Permission> {
        match self {
            Permission::TokensManage => Some(Permission::TokensView),
            Permission::ActionsPropose | Permission::ActionsApprove => Some(Permission::ActionsView),
            Permission::MembersManage | Permission::RolesManage => Some(Permission::MembersView),
            _ => None,
        }
    }
}

/// Adds implied permissions.
pub fn normalise(mut set: PermissionSet) -> PermissionSet {
    let implied: Vec<Permission> = set.iter().filter_map(|p| p.implies()).collect();
    set.extend(implied);
    set
}

/// Input from a client: every entry must be a known permission.
pub fn parse_set(items: &[String]) -> Result<PermissionSet, &'static str> {
    if items.len() > Permission::ALL.len() * 4 {
        return Err("too many permissions");
    }
    let set = items.iter().map(|s| Permission::parse(s).ok_or("unknown permission")).collect::<Result<_, _>>()?;
    Ok(normalise(set))
}

/// Stored values: unknown entries (e.g. from a newer version) are never granted.
pub fn from_stored(items: &[String]) -> PermissionSet {
    normalise(items.iter().filter_map(|s| Permission::parse(s)).collect())
}

pub fn to_strings(set: &PermissionSet) -> Vec<String> {
    set.iter().map(|p| p.as_str().to_owned()).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Owner,
    Admin,
    Operator,
    Viewer,
}

impl Builtin {
    pub const ALL: [Builtin; 4] = [Builtin::Owner, Builtin::Admin, Builtin::Operator, Builtin::Viewer];

    pub fn key(self) -> &'static str {
        match self {
            Builtin::Owner => "owner",
            Builtin::Admin => "admin",
            Builtin::Operator => "operator",
            Builtin::Viewer => "viewer",
        }
    }

    pub fn permissions(self) -> PermissionSet {
        use Permission::*;
        match self {
            Builtin::Owner | Builtin::Admin => Permission::ALL.into_iter().collect(),
            Builtin::Operator => [TelemetryView, TokensView, ActionsView, ActionsPropose, MembersView].into(),
            Builtin::Viewer => [TelemetryView, ActionsView, MembersView].into(),
        }
    }
}

/// The member making a change, with their effective permissions in the project.
#[derive(Debug, Clone)]
pub struct Actor {
    pub user_id: Uuid,
    pub permissions: PermissionSet,
    pub is_owner: bool,
}

impl Actor {
    pub fn has(&self, p: Permission) -> bool {
        self.permissions.contains(&p)
    }
}

/// The member being changed.
#[derive(Debug, Clone)]
pub struct Target {
    pub user_id: Uuid,
    pub is_owner: bool,
    /// The member's current effective permissions.
    pub permissions: PermissionSet,
}

#[derive(Debug, Clone)]
pub enum Grant {
    /// A role template; `owner` for the built-in Owner role.
    Role {
        permissions: PermissionSet,
        owner: bool,
    },
    Individual(PermissionSet),
    Remove,
}

impl Grant {
    fn permissions(&self) -> Option<&PermissionSet> {
        match self {
            Grant::Role { permissions, .. } | Grant::Individual(permissions) => Some(permissions),
            Grant::Remove => None,
        }
    }

    fn makes_owner(&self) -> bool {
        matches!(self, Grant::Role { owner: true, .. })
    }
}

/// May `actor` apply `grant` to `target`? `owners` is the project's current number of owners.
pub fn check_member_change(actor: &Actor, target: &Target, grant: &Grant, owners: i64) -> Result<(), &'static str> {
    if actor.user_id == target.user_id {
        return Err("you cannot change your own membership");
    }
    if !actor.has(Permission::MembersManage) {
        return Err("requires the members.manage permission");
    }
    if target.is_owner && !actor.is_owner {
        return Err("only an owner can change another owner");
    }
    if !actor.is_owner && !target.permissions.is_subset(&actor.permissions) {
        return Err("you cannot change a member who has permissions you do not have");
    }
    if grant.makes_owner() && !actor.is_owner {
        return Err("only an owner can grant the Owner role");
    }
    if let Some(p) = grant.permissions()
        && !p.is_subset(&actor.permissions)
    {
        return Err("you can only grant permissions you have yourself");
    }
    if target.is_owner && !grant.makes_owner() && owners <= 1 {
        return Err("a project needs at least one owner");
    }
    Ok(())
}

/// May `actor` hand out a role through an invite link? `owner` is the built-in Owner role.
/// Checked when the link is created and again, against the creator's current membership, when
/// it is accepted.
pub fn check_invite(actor: &Actor, role: &PermissionSet, owner: bool) -> Result<(), &'static str> {
    if !actor.has(Permission::MembersManage) {
        return Err("requires the members.manage permission");
    }
    if owner && !actor.is_owner {
        return Err("only an owner can invite with the Owner role");
    }
    if !role.is_subset(&actor.permissions) {
        return Err("you can only invite with permissions you have yourself");
    }
    Ok(())
}

/// May `actor` create (`old = None`), edit or delete (`new == old`) a role template?
pub fn check_template_change(
    actor: &Actor,
    old: Option<&PermissionSet>,
    new: &PermissionSet,
) -> Result<(), &'static str> {
    if !actor.has(Permission::RolesManage) {
        return Err("requires the roles.manage permission");
    }
    if old.is_some_and(|o| !o.is_subset(&actor.permissions)) {
        return Err("you cannot change a role that has permissions you do not have");
    }
    if !new.is_subset(&actor.permissions) {
        return Err("a role can only contain permissions you have yourself");
    }
    Ok(())
}

/// A catalog entry for clients.
#[derive(Debug, Serialize, ToSchema)]
pub struct PermissionInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub group: &'static str,
}

pub fn catalog() -> Vec<PermissionInfo> {
    Permission::ALL.into_iter().map(|p| PermissionInfo { id: p.as_str(), label: p.label(), group: p.group() }).collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use Permission::*;
    use proptest::prelude::*;

    fn set(ps: &[Permission]) -> PermissionSet {
        ps.iter().copied().collect()
    }

    fn actor(ps: &[Permission], is_owner: bool) -> Actor {
        Actor { user_id: Uuid::from_u128(1), permissions: normalise(set(ps)), is_owner }
    }

    fn target(is_owner: bool) -> Target {
        let permissions = if is_owner { Builtin::Owner.permissions() } else { PermissionSet::new() };
        Target { user_id: Uuid::from_u128(2), is_owner, permissions }
    }

    #[test]
    fn permissions_parse_exactly_and_round_trip() {
        for p in Permission::ALL {
            assert_eq!(Permission::parse(p.as_str()), Some(p));
        }
        for bad in ["", "Telemetry.view", " telemetry.view", "telemetry.view ", "*", "telemetry.*", "admin", "owner"] {
            assert_eq!(Permission::parse(bad), None, "{bad:?}");
        }
        let ids: BTreeSet<&str> = Permission::ALL.iter().map(|p| p.as_str()).collect();
        assert_eq!(ids.len(), Permission::ALL.len(), "ids are unique");
    }

    #[test]
    fn client_input_is_strict_and_stored_values_are_lenient() {
        assert_eq!(parse_set(&["telemetry.view".into()]).unwrap(), set(&[TelemetryView]));
        assert_eq!(parse_set(&["telemetry.view".into(), "root".into()]), Err("unknown permission"));
        assert_eq!(parse_set(&vec!["telemetry.view".to_owned(); 100]), Err("too many permissions"));
        assert_eq!(from_stored(&["telemetry.view".into(), "future.thing".into()]), set(&[TelemetryView]));
        assert!(from_stored(&[]).is_empty());
    }

    #[test]
    fn managing_implies_viewing() {
        assert_eq!(normalise(set(&[TokensManage])), set(&[TokensManage, TokensView]));
        assert_eq!(normalise(set(&[ActionsApprove])), set(&[ActionsApprove, ActionsView]));
        assert_eq!(normalise(set(&[RolesManage])), set(&[RolesManage, MembersView]));
        assert_eq!(normalise(set(&[TelemetryView])), set(&[TelemetryView]));
    }

    #[test]
    fn builtin_roles_match_the_spec() {
        assert_eq!(Builtin::Owner.permissions().len(), Permission::ALL.len());
        assert_eq!(Builtin::Admin.permissions(), Builtin::Owner.permissions());
        assert_eq!(
            Builtin::Operator.permissions(),
            set(&[TelemetryView, TokensView, ActionsView, ActionsPropose, MembersView])
        );
        assert_eq!(Builtin::Viewer.permissions(), set(&[TelemetryView, ActionsView, MembersView]));
        for b in Builtin::ALL {
            assert_eq!(normalise(b.permissions()), b.permissions(), "{b:?} is closed under implication");
        }
        let keys: BTreeSet<&str> = Builtin::ALL.iter().map(|b| b.key()).collect();
        assert_eq!(keys, ["admin", "operator", "owner", "viewer"].into(), "keys match the SQL CHECK");
    }

    #[test]
    fn builtin_roles_in_the_migration_match_the_code() {
        let sql = include_str!("../../../infra/migrations/postgres/0004_roles.sql");
        for b in Builtin::ALL {
            let name = format!("{}{}", b.key()[..1].to_ascii_uppercase(), &b.key()[1..]);
            let start = sql.find(&format!("'{name}', ARRAY[")).unwrap_or_else(|| panic!("{name} missing"));
            let rest = &sql[start..];
            let body = &rest[rest.find('[').unwrap() + 1..rest.find(']').unwrap()];
            let ids: Vec<String> = body.split(',').map(|s| s.trim().trim_matches('\'').to_owned()).collect();
            assert_eq!(parse_set(&ids).unwrap(), b.permissions(), "{name}");
            assert!(rest[..rest.find(')').unwrap()].contains(&format!("'{}'", b.key())), "{name} builtin key");
        }
    }

    #[test]
    fn nobody_changes_their_own_membership() {
        let a = actor(&Permission::ALL, true);
        let me = Target { user_id: a.user_id, is_owner: true, permissions: Builtin::Owner.permissions() };
        assert_eq!(
            check_member_change(&a, &me, &Grant::Individual(set(&[TelemetryView])), 5),
            Err("you cannot change your own membership")
        );
        assert_eq!(check_member_change(&a, &me, &Grant::Remove, 5), Err("you cannot change your own membership"));
    }

    #[test]
    fn grants_are_limited_to_the_actors_own_permissions() {
        let manager = actor(&[MembersManage, TelemetryView], false);
        let ok = Grant::Individual(set(&[TelemetryView, MembersView]));
        assert_eq!(check_member_change(&manager, &target(false), &ok, 1), Ok(()));
        let more = Grant::Individual(set(&[TelemetryView, TokensManage]));
        assert_eq!(
            check_member_change(&manager, &target(false), &more, 1),
            Err("you can only grant permissions you have yourself")
        );
        let admin_role = Grant::Role { permissions: Builtin::Admin.permissions(), owner: false };
        assert!(check_member_change(&manager, &target(false), &admin_role, 1).is_err());
        let viewer = actor(&[TelemetryView, MembersView], false);
        assert_eq!(
            check_member_change(&viewer, &target(false), &Grant::Remove, 1),
            Err("requires the members.manage permission")
        );
    }

    #[test]
    fn stronger_members_are_out_of_reach() {
        let manager = actor(&[MembersManage, TelemetryView], false);
        let admin = Target { user_id: Uuid::from_u128(2), is_owner: false, permissions: Builtin::Admin.permissions() };
        let msg = Err("you cannot change a member who has permissions you do not have");
        assert_eq!(check_member_change(&manager, &admin, &Grant::Remove, 1), msg, "no removing an admin");
        assert_eq!(check_member_change(&manager, &admin, &Grant::Individual(set(&[TelemetryView])), 1), msg);
        let peer = Target { user_id: Uuid::from_u128(2), is_owner: false, permissions: set(&[TelemetryView]) };
        assert_eq!(check_member_change(&manager, &peer, &Grant::Remove, 1), Ok(()));
        let owner = actor(&Permission::ALL, true);
        assert_eq!(check_member_change(&owner, &admin, &Grant::Remove, 1), Ok(()));
    }

    #[test]
    fn only_owners_touch_owners() {
        let admin = actor(&Permission::ALL, false);
        let owner_role = Grant::Role { permissions: Builtin::Owner.permissions(), owner: true };
        assert_eq!(
            check_member_change(&admin, &target(false), &owner_role, 1),
            Err("only an owner can grant the Owner role")
        );
        assert_eq!(
            check_member_change(&admin, &target(true), &Grant::Remove, 2),
            Err("only an owner can change another owner")
        );
        let owner = actor(&Permission::ALL, true);
        assert_eq!(check_member_change(&owner, &target(false), &owner_role, 1), Ok(()));
        assert_eq!(check_member_change(&owner, &target(true), &Grant::Remove, 2), Ok(()));
    }

    #[test]
    fn the_last_owner_stays() {
        let owner = actor(&Permission::ALL, true);
        let admin_role = Grant::Role { permissions: Builtin::Admin.permissions(), owner: false };
        assert_eq!(
            check_member_change(&owner, &target(true), &admin_role, 1),
            Err("a project needs at least one owner")
        );
        assert_eq!(
            check_member_change(&owner, &target(true), &Grant::Remove, 1),
            Err("a project needs at least one owner")
        );
        assert_eq!(check_member_change(&owner, &target(true), &admin_role, 2), Ok(()));
        let owner_role = Grant::Role { permissions: Builtin::Owner.permissions(), owner: true };
        assert_eq!(check_member_change(&owner, &target(true), &owner_role, 1), Ok(()), "no-op on the owner is fine");
    }

    #[test]
    fn invites_follow_the_same_rules_as_assigning_a_role() {
        let manager = actor(&[MembersManage, TelemetryView], false);
        assert_eq!(check_invite(&manager, &set(&[TelemetryView, MembersView]), false), Ok(()));
        assert_eq!(
            check_invite(&manager, &Builtin::Operator.permissions(), false),
            Err("you can only invite with permissions you have yourself")
        );
        let admin = actor(&Permission::ALL, false);
        assert_eq!(check_invite(&admin, &Builtin::Admin.permissions(), false), Ok(()));
        assert_eq!(
            check_invite(&admin, &Builtin::Owner.permissions(), true),
            Err("only an owner can invite with the Owner role")
        );
        assert_eq!(check_invite(&actor(&Permission::ALL, true), &Builtin::Owner.permissions(), true), Ok(()));
        let viewer = actor(&[TelemetryView, MembersView], false);
        assert_eq!(check_invite(&viewer, &set(&[]), false), Err("requires the members.manage permission"));
    }

    #[test]
    fn templates_never_exceed_the_editor() {
        let manager = actor(&[RolesManage, TelemetryView, TokensView], false);
        assert_eq!(check_template_change(&manager, None, &set(&[TelemetryView, TokensView])), Ok(()));
        assert_eq!(
            check_template_change(&manager, None, &set(&[TokensManage])),
            Err("a role can only contain permissions you have yourself")
        );
        let admin_perms = Builtin::Admin.permissions();
        assert_eq!(
            check_template_change(&manager, Some(&admin_perms), &set(&[TelemetryView])),
            Err("you cannot change a role that has permissions you do not have"),
            "cannot strip a stronger role either"
        );
        let viewer = actor(&[TelemetryView, MembersView], false);
        assert_eq!(check_template_change(&viewer, None, &set(&[])), Err("requires the roles.manage permission"));
    }

    #[test]
    fn catalog_lists_every_permission_once() {
        let c = catalog();
        assert_eq!(c.len(), Permission::ALL.len());
        assert!(c.iter().all(|e| !e.label.is_empty() && !e.group.is_empty()));
    }

    fn arb_set() -> impl Strategy<Value = PermissionSet> {
        proptest::collection::btree_set(proptest::sample::select(Permission::ALL.to_vec()), 0..=Permission::ALL.len())
    }

    #[derive(Debug, Clone)]
    enum Op {
        Individual(PermissionSet),
        Role(PermissionSet, bool),
        Remove,
    }

    fn arb_op() -> impl Strategy<Value = Op> {
        prop_oneof![
            arb_set().prop_map(Op::Individual),
            (arb_set(), any::<bool>()).prop_map(|(s, o)| Op::Role(s, o)),
            Just(Op::Remove),
        ]
    }

    proptest! {
        #[test]
        fn accepted_grants_are_always_subsets_of_the_actor(a in arb_set(), g in arb_set(), owner in any::<bool>()) {
            let actor = Actor { user_id: Uuid::from_u128(1), permissions: normalise(a), is_owner: owner };
            let g = normalise(g);
            if check_member_change(&actor, &target(false), &Grant::Individual(g.clone()), 1).is_ok() {
                prop_assert!(g.is_subset(&actor.permissions));
            }
        }

        #[test]
        fn accepted_invites_never_exceed_the_creator(a in arb_set(), r in arb_set(), a_owner in any::<bool>(), r_owner in any::<bool>()) {
            let creator = Actor { user_id: Uuid::from_u128(1), permissions: normalise(a), is_owner: a_owner };
            let role = normalise(r);
            if check_invite(&creator, &role, r_owner).is_ok() {
                prop_assert!(role.is_subset(&creator.permissions));
                prop_assert!(creator.has(Permission::MembersManage));
                prop_assert!(!r_owner || a_owner);
            }
        }

        /// Members start as one owner plus four members without permissions; random changes by random
        /// members never hand out a permission the acting member lacked, and an owner always remains.
        #[test]
        fn no_sequence_of_changes_escalates(ops in proptest::collection::vec((0usize..5, 0usize..5, arb_op()), 0..60)) {
            let mut members: Vec<Option<(PermissionSet, bool)>> =
                vec![Some((Builtin::Owner.permissions(), true)), Some((PermissionSet::new(), false)),
                     Some((PermissionSet::new(), false)), Some((PermissionSet::new(), false)), Some((PermissionSet::new(), false))];
            for (ai, ti, op) in ops {
                let (Some((a_perms, a_owner)), Some((t_perms, t_owner))) = (members[ai].clone(), members[ti].clone()) else { continue };
                let actor = Actor { user_id: Uuid::from_u128(ai as u128), permissions: a_perms.clone(), is_owner: a_owner };
                let t = Target { user_id: Uuid::from_u128(ti as u128), is_owner: t_owner, permissions: t_perms.clone() };
                let owners = members.iter().flatten().filter(|m| m.1).count() as i64;
                let grant = match &op {
                    Op::Individual(s) => Grant::Individual(normalise(s.clone())),
                    Op::Role(s, true) => Grant::Role { permissions: Builtin::Owner.permissions().union(s).copied().collect(), owner: true },
                    Op::Role(s, false) => Grant::Role { permissions: normalise(s.clone()), owner: false },
                    Op::Remove => Grant::Remove,
                };
                if check_member_change(&actor, &t, &grant, owners).is_err() { continue; }
                prop_assert!(a_owner || t_perms.is_subset(&a_perms), "only weaker-or-equal members are changed");
                members[ti] = match grant {
                    Grant::Individual(p) => { prop_assert!(p.is_subset(&a_perms)); Some((p, false)) }
                    Grant::Role { permissions, owner } => {
                        prop_assert!(permissions.is_subset(&a_perms));
                        prop_assert!(!owner || a_owner);
                        Some((permissions, owner))
                    }
                    Grant::Remove => None,
                };
                prop_assert!(members.iter().flatten().any(|m| m.1), "an owner always remains");
            }
        }
    }
}
