//! Permissions, built-in roles and the rules that keep anyone from granting more than they
//! have. The same rules govern project members (`Permission`) and ControlHub administrators
//! (`crate::system_permissions::SystemPermission`). Pure (no I/O): handlers load the actor and
//! target, these functions decide.

use std::collections::BTreeSet;
use std::fmt::Debug;

use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

/// Error texts of the rules, worded for one kind of permission.
#[derive(Debug)]
pub struct RuleText {
    pub own_change: &'static str,
    pub needs_members_manage: &'static str,
    pub owner_changes_owner: &'static str,
    pub stronger_target: &'static str,
    pub owner_grants_owner: &'static str,
    pub grant_exceeds_actor: &'static str,
    pub last_owner: &'static str,
    pub owner_invites_owner: &'static str,
    pub invite_exceeds_actor: &'static str,
    pub needs_roles_manage: &'static str,
    pub stronger_template: &'static str,
    pub template_exceeds_actor: &'static str,
}

/// A family of permissions with its own catalog, implications and rule texts.
pub trait PermissionKind: Copy + Ord + Debug + 'static {
    const ALL: &'static [Self];
    /// Needed to change members (and to create invites).
    const MEMBERS_MANAGE: Self;
    /// Needed to change role templates.
    const ROLES_MANAGE: Self;
    const TEXT: RuleText;

    fn id(self) -> &'static str;
    fn title(self) -> &'static str;
    fn section(self) -> &'static str;
    /// A permission this one cannot work without.
    fn requires(self) -> Option<Self>;

    /// Exact match only: no trimming, no case folding, no wildcards.
    fn from_id(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|p| p.id() == s)
    }
}

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

impl PermissionKind for Permission {
    const ALL: &'static [Self] = &Permission::ALL;
    const MEMBERS_MANAGE: Self = Permission::MembersManage;
    const ROLES_MANAGE: Self = Permission::RolesManage;
    const TEXT: RuleText = RuleText {
        own_change: "you cannot change your own membership",
        needs_members_manage: "requires the members.manage permission",
        owner_changes_owner: "only an owner can change another owner",
        stronger_target: "you cannot change a member who has permissions you do not have",
        owner_grants_owner: "only an owner can grant the Owner role",
        grant_exceeds_actor: "you can only grant permissions you have yourself",
        last_owner: "a project needs at least one owner",
        owner_invites_owner: "only an owner can invite with the Owner role",
        invite_exceeds_actor: "you can only invite with permissions you have yourself",
        needs_roles_manage: "requires the roles.manage permission",
        stronger_template: "you cannot change a role that has permissions you do not have",
        template_exceeds_actor: "a role can only contain permissions you have yourself",
    };

    fn id(self) -> &'static str {
        self.as_str()
    }
    fn title(self) -> &'static str {
        self.label()
    }
    fn section(self) -> &'static str {
        self.group()
    }
    fn requires(self) -> Option<Self> {
        self.implies()
    }
}

/// Adds implied permissions.
pub fn normalise<P: PermissionKind>(mut set: BTreeSet<P>) -> BTreeSet<P> {
    let implied: Vec<P> = set.iter().filter_map(|p| p.requires()).collect();
    set.extend(implied);
    set
}

/// Input from a client: every entry must be a known permission.
pub fn parse_set(items: &[String]) -> Result<PermissionSet, &'static str> {
    parse_set_of(items)
}

pub fn parse_set_of<P: PermissionKind>(items: &[String]) -> Result<BTreeSet<P>, &'static str> {
    if items.len() > P::ALL.len() * 4 {
        return Err("too many permissions");
    }
    let set = items.iter().map(|s| P::from_id(s).ok_or("unknown permission")).collect::<Result<_, _>>()?;
    Ok(normalise(set))
}

/// Stored values: unknown entries (e.g. from a newer version) are never granted.
pub fn from_stored(items: &[String]) -> PermissionSet {
    from_stored_of(items)
}

pub fn from_stored_of<P: PermissionKind>(items: &[String]) -> BTreeSet<P> {
    normalise(items.iter().filter_map(|s| P::from_id(s)).collect())
}

pub fn to_strings<P: PermissionKind>(set: &BTreeSet<P>) -> Vec<String> {
    set.iter().map(|p| p.id().to_owned()).collect()
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

/// The member making a change, with their effective permissions. `is_owner`: the top role
/// (project Owner, ControlHub Super Admin).
#[derive(Debug, Clone)]
pub struct Actor<P: PermissionKind = Permission> {
    pub user_id: Uuid,
    pub permissions: BTreeSet<P>,
    pub is_owner: bool,
}

impl<P: PermissionKind> Actor<P> {
    pub fn has(&self, p: P) -> bool {
        self.permissions.contains(&p)
    }
}

/// The member being changed.
#[derive(Debug, Clone)]
pub struct Target<P: PermissionKind = Permission> {
    pub user_id: Uuid,
    pub is_owner: bool,
    /// The member's current effective permissions.
    pub permissions: BTreeSet<P>,
}

#[derive(Debug, Clone)]
pub enum Grant<P: PermissionKind = Permission> {
    /// A role template; `owner` for the top built-in role.
    Role {
        permissions: BTreeSet<P>,
        owner: bool,
    },
    Individual(BTreeSet<P>),
    Remove,
}

impl<P: PermissionKind> Grant<P> {
    fn permissions(&self) -> Option<&BTreeSet<P>> {
        match self {
            Grant::Role { permissions, .. } | Grant::Individual(permissions) => Some(permissions),
            Grant::Remove => None,
        }
    }

    fn makes_owner(&self) -> bool {
        matches!(self, Grant::Role { owner: true, .. })
    }
}

/// May `actor` apply `grant` to `target`? `owners` is the current number of top-role holders.
pub fn check_member_change<P: PermissionKind>(
    actor: &Actor<P>,
    target: &Target<P>,
    grant: &Grant<P>,
    owners: i64,
) -> Result<(), &'static str> {
    let t = &P::TEXT;
    if actor.user_id == target.user_id {
        return Err(t.own_change);
    }
    if !actor.has(P::MEMBERS_MANAGE) {
        return Err(t.needs_members_manage);
    }
    if target.is_owner && !actor.is_owner {
        return Err(t.owner_changes_owner);
    }
    if !actor.is_owner && !target.permissions.is_subset(&actor.permissions) {
        return Err(t.stronger_target);
    }
    if grant.makes_owner() && !actor.is_owner {
        return Err(t.owner_grants_owner);
    }
    if let Some(p) = grant.permissions()
        && !p.is_subset(&actor.permissions)
    {
        return Err(t.grant_exceeds_actor);
    }
    if target.is_owner && !grant.makes_owner() && owners <= 1 {
        return Err(t.last_owner);
    }
    Ok(())
}

/// May `actor` hand out a role through an invite link? `owner` is the built-in Owner role.
/// Checked when the link is created and again, against the creator's current membership, when
/// it is accepted.
pub fn check_invite<P: PermissionKind>(actor: &Actor<P>, role: &BTreeSet<P>, owner: bool) -> Result<(), &'static str> {
    let t = &P::TEXT;
    if !actor.has(P::MEMBERS_MANAGE) {
        return Err(t.needs_members_manage);
    }
    if owner && !actor.is_owner {
        return Err(t.owner_invites_owner);
    }
    if !role.is_subset(&actor.permissions) {
        return Err(t.invite_exceeds_actor);
    }
    Ok(())
}

/// May `actor` create (`old = None`), edit or delete (`new == old`) a role template?
pub fn check_template_change<P: PermissionKind>(
    actor: &Actor<P>,
    old: Option<&BTreeSet<P>>,
    new: &BTreeSet<P>,
) -> Result<(), &'static str> {
    let t = &P::TEXT;
    if !actor.has(P::ROLES_MANAGE) {
        return Err(t.needs_roles_manage);
    }
    if old.is_some_and(|o| !o.is_subset(&actor.permissions)) {
        return Err(t.stronger_template);
    }
    if !new.is_subset(&actor.permissions) {
        return Err(t.template_exceeds_actor);
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
    catalog_of::<Permission>()
}

pub fn catalog_of<P: PermissionKind>() -> Vec<PermissionInfo> {
    P::ALL.iter().map(|p| PermissionInfo { id: p.id(), label: p.title(), group: p.section() }).collect()
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
            assert_eq!(Permission::from_id(p.as_str()), Some(p));
        }
        for bad in ["", "Telemetry.view", " telemetry.view", "telemetry.view ", "*", "telemetry.*", "admin", "owner"] {
            assert_eq!(Permission::from_id(bad), None, "{bad:?}");
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
