# Graph Report - control_hub  (2026-10-01)

## Corpus Check
- 383 files · ~256,168 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 14 file(s) not represented in the graph (top: .mdc 4, (none) 4, .toml 3)

## Summary
- 4601 nodes · 11452 edges · 201 communities (187 shown, 14 thin omitted)
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 246 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `25270509`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- .pool
- http.rs
- TimelineTracksView.tsx
- convert.rs
- CurrentUser
- unwrap
- tests.rs
- Workspace.tsx
- robustness_tests.rs
- adapter.rs
- useWorkspaceEngine.ts
- permissions.rs
- broker.rs
- scripts/lib.mjs
- types.rs
- Icon
- edge-path.ts
- mcp.rs
- tokens.rs
- map-key.ts
- Session
- InvitesTab.tsx
- controlhub-actions/src/lib.rs
- controlhub-api/src/auth.rs
- roles.rs
- web/package.json
- generated/index.ts
- client.rs
- signup.rs
- State
- Permission
- controlhub-api/src/main.rs
- invites.rs
- events.rs
- controlhub-processor/src/main.rs
- Engine
- controlhub-ingest/src/auth.rs
- session.rs
- grpc.rs
- telemetry.rs
- PgStore
- ch.rs
- IntegrationGuide.tsx
- layout.rs
- mcp.test.mjs
- sanitize.rs
- plan.rs
- PixiMap
- model.rs
- Timeline
- secrets.rs
- authorize
- bootstrap.rs
- redaction.rs
- ChStore
- SecurityPage.tsx
- engine.worker.ts
- pipeline.rs
- String
- doctor.rs
- edgeStyleStore.ts
- tracks.rs
- reducer.rs
- SettingsPage.tsx
- .new
- controlhub-runtime/src/lib.rs
- verify.rs
- ApiError
- draw.ts
- ActionPlanRow
- routes/actions.rs
- discover.rs
- String
- context.rs
- ref_node_assert
- ActionsPage.tsx
- vitest
- SystemPermission
- ids.rs
- WebEngine
- common.cjs
- integration/lib.mjs
- doctor.mjs
- test_fixtures.rs
- pg.rs
- controlhub-executor/src/main.rs
- live.rs
- controlhub-engine-core/src/lib.rs
- sdk-node/package.json
- proxy.ts
- testing.rs
- system_permissions.rs
- controlhub-cli/src/main.rs
- Event
- .from_request_parts
- TimelineBar.tsx
- controlhub-web-engine/src/lib.rs
- mod.rs
- nonempty
- pixi-map.ts
- index.js
- dev.mjs
- WorkspaceHost.tsx
- LoginThrottle
- logs
- demo-system/package.json
- api-client/package.json
- compilerOptions
- .apply_batch
- EventsClient
- @playwright/test
- helpers.ts
- Prose
- external.ts
- precise-start.test.js
- seo.test.ts
- test_http.rs
- skew.rs
- Scope
- Permissions and project roles — design
- web-engine/package.json
- e2e/package.json
- api-surface.test.mjs
- Action
- Command
- EntityKind
- check_member_change
- check
- dependencies
- collectErrors
- File map
- compilerOptions
- bench.rs
- Builtin
- state.rs
- ADR-0011: Read-only API tokens and the WebSocket live channel
- package.json
- controlhub-domain
- Splitter.tsx
- openapi.rs
- AuthError
- super
- adr/README.md
- README.md
- Commands
- orders.cjs
- gateway.cjs
- Onboarding.tsx
- Map routing, service focus and replay range — design
- index.d.ts
- ControlHub — tasks
- randomHex
- grid.ts
- Scope
- Security model (current implementation)
- Layered, zoomable timeline — design
- ref_node_http
- config.test.js
- invites.spec.ts
- web-engine-contract.test.ts
- live
- controlhub-ingest/src/main.rs
- Local development and operations
- node-http-app/package.json
- cull.ts
- common.test.mjs
- e2e/tsconfig.json
- AGENTS.md — working in the ControlHub repo
- src/app/layout.tsx
- sanitize_traces
- ADR-0001: Monorepo layout
- ADR-0002: Kafka client — rskafka
- ADR-0003: Consumer offsets in PostgreSQL, at-least-once
- ADR-0004: ClickHouse dedup via ReplacingMergeTree
- ADR-0005: WASM engine in a Worker, content-hashed artifact
- ADR-0009: The Worker fetches JSON and hands raw text to WASM
- ADR-0012: Controlled actions — plans, policy, four-eyes approval, separate executor
- ADR-0013: Read-only MCP server as a CLI subcommand over the REST API
- ADR-0014: OTLP metrics are not ingested (yet); service metrics come from spans
- Invite links, personal account and onboarding — design
- fault.mjs
- loadgen.mjs
- ControlHub
- StepState
- reference.rs
- methods_delegate_to_the_engine
- ADR-0007: Browser authentication
- ADR-0008: TypeScript 5.9
- api-client/tsconfig.json
- web-engine/tsconfig.json
- AI integration
- web/AGENTS.md
- postcss.config.mjs
- controlhub-cli
- MemberChange
- Signal
- (public)/page.tsx

## God Nodes (most connected - your core abstractions)
1. `State` - 93 edges
2. `CurrentUser` - 92 edges
3. `PgStore` - 71 edges
4. `Engine` - 60 edges
5. `unwrap()` - 60 edges
6. `Icon()` - 54 edges
7. `send()` - 41 edges
8. `vitest` - 37 edges
9. `ApiError` - 37 edges
10. `Permission` - 37 edges

## Surprising Connections (you probably didn't know these)
- `Architecture` --references--> `TimelineBar()`  [INFERRED]
  docs/superpowers/specs/2026-09-29-layered-timeline-design.md → apps/web/src/components/workspace/TimelineBar.tsx
- `Task 5: Web — permissions helper and gating` --references--> `can()`  [INFERRED]
  docs/superpowers/plans/2026-09-30-permissions-roles.md → apps/web/src/lib/permissions.ts
- `Rules` --references--> `unwrap()`  [INFERRED]
  AGENTS.md → packages/api-client/src/index.ts
- `Global Constraints` --references--> `unwrap()`  [INFERRED]
  docs/superpowers/plans/2026-09-30-permissions-roles.md → packages/api-client/src/index.ts
- `Task 1: Permission model (pure)` --references--> `normalise()`  [INFERRED]
  docs/superpowers/plans/2026-09-30-permissions-roles.md → apps/web/src/components/settings/matrix.ts

## Import Cycles
- 1-file cycle: `crates/controlhub-api/src/system_permissions.rs -> crates/controlhub-api/src/system_permissions.rs`
- 1-file cycle: `crates/controlhub-api/src/permissions.rs -> crates/controlhub-api/src/permissions.rs`

## Communities (201 total, 14 thin omitted)

### Community 0 - ".pool"
Cohesion: 0.14
Nodes (6): AdminAuditRow, AdminProjectRow, AdminUserRow, an_empty_system_role_prefix_is_refused_before_touching_the_database(), PgStore, SystemGrantRow

### Community 1 - "http.rs"
Cohesion: 0.05
Nodes (69): accept_error_response(), accept_errors_map_to_otlp_status_codes(), accepted_traces_are_sanitised_and_scoped_by_the_token(), all_invalid_spans_are_not_published(), auth_header(), auth_is_checked_before_the_body_is_parsed(), bad_span(), batch_with_no_valid_events_is_400_and_not_published() (+61 more)

### Community 2 - "TimelineTracksView.tsx"
Cohesion: 0.12
Nodes (39): ActionBar, firstItemStart(), MIN_LOOP_NS, placeAbove(), Placement, playStart(), rangeBetween(), replayStep() (+31 more)

### Community 3 - "convert.rs"
Cohesion: 0.05
Nodes (56): any_to_string(), any_values_are_stringified_deterministically(), anyv(), attr(), attributes_without_values_become_empty_strings(), clamp_ns(), convert(), converted_extend_and_is_empty() (+48 more)

### Community 4 - "CurrentUser"
Cohesion: 0.11
Nodes (63): CurrentUser, from_stored_of(), account_target(), active_user_by_email(), add_member(), AdminAuditEntryDto, AdminAuditQuery, administrator_dtos_use_system_permissions_and_mark_the_super_admin() (+55 more)

### Community 5 - "unwrap"
Cohesion: 0.12
Nodes (40): Admin(), Sessions(), accountLock(), ADMIN_TAB_LABELS, AdminTab, adminTabs(), auditActionLabel(), canAdmin() (+32 more)

### Community 6 - "tests.rs"
Cohesion: 0.06
Nodes (59): an_inbound_request_arrives_before_its_service_calls_anything(), arb_items(), BASE, bounded_window_evicts_oldest(), chain_engine(), chain_items(), clock_skew_is_clamped_and_flagged(), clock_skew_shifts_the_callee_and_its_subtree_for_display() (+51 more)

### Community 7 - "Workspace.tsx"
Cohesion: 0.11
Nodes (42): StatusRow(), EventsPanel(), Log, LogsPanel(), SEVERITIES, severityClass(), PanelScope, SignalsPanel() (+34 more)

### Community 8 - "robustness_tests.rs"
Cohesion: 0.06
Nodes (46): VisibleState, applying_the_same_batch_twice_is_idempotent(), arb_ops(), arb_world(), BASE, changed_nodes_report_status_transitions_only(), check_frame(), compare_is_symmetric_and_empty_for_equal_times() (+38 more)

### Community 9 - "adapter.rs"
Cohesion: 0.13
Nodes (14): a_readback_that_differs_is_a_failure(), cfg(), clear_posts_zeroes(), COMPOSE_PROJECT, COMPOSE_SERVICE, Demo, demo_at(), DemoConfig (+6 more)

### Community 10 - "useWorkspaceEngine.ts"
Cohesion: 0.06
Nodes (37): CompareView(), Inspector(), Metric(), relations(), STATUS_CLASS, STATUS_TONE, LINE_STYLES, Props (+29 more)

### Community 11 - "permissions.rs"
Cohesion: 0.17
Nodes (14): catalog(), catalog_lists_every_permission_once(), catalog_of(), Grant<P>, grants_are_limited_to_the_actors_own_permissions(), invites_follow_the_same_rules_as_assigning_a_role(), nobody_changes_their_own_membership(), only_owners_touch_owners() (+6 more)

### Community 12 - "broker.rs"
Cohesion: 0.09
Nodes (30): acked_publish_returns_offset_and_releases_queue_bytes(), broker_failure_is_an_error_never_success(), byte_budget_rejects_when_exhausted(), cfg(), default_config_is_bounded(), dropped_responder_is_unavailable(), FakeBroker, full_queue_is_overloaded_not_buffered() (+22 more)

### Community 13 - "scripts/lib.mjs"
Cohesion: 0.07
Nodes (45): defaults, dir, infraPath, owner, services, bytes, genDir, pkg (+37 more)

### Community 14 - "types.rs"
Cohesion: 0.08
Nodes (25): Batch, Diagnostics, EngineConfig, EventItem, EventMarker, Filters, FrameSummary, LogItem (+17 more)

### Community 15 - "Icon"
Cohesion: 0.14
Nodes (31): Account(), Profile(), Projects(), AppIndex(), Integration(), nav, PublicLayout(), AccountHome() (+23 more)

### Community 16 - "edge-path.ts"
Cohesion: 0.11
Nodes (29): build(), CardBox, cardExit(), curvedPoints(), DEFAULT_EDGE_STYLE, EDGE_STYLES, edgePath, EdgeStyle (+21 more)

### Community 17 - "mcp.rs"
Cohesion: 0.09
Nodes (31): api_failures_become_tool_errors_without_the_token(), arguments_are_validated_before_any_request(), call_ok(), error(), every_argument_type_is_checked(), exchange(), handshake_and_tool_list_are_read_only(), INSTRUCTIONS (+23 more)

### Community 18 - "tokens.rs"
Cohesion: 0.12
Nodes (31): audit(), create(), CreatedTokenResponse, CreateTokenRequest, days_from_now(), DEFAULT_API_TOKEN_DAYS, expiry(), expiry_bounds_per_kind() (+23 more)

### Community 19 - "map-key.ts"
Cohesion: 0.14
Nodes (22): KeyItem, KeySection, MAP_KEY, [NO_DATA, HEALTHY, DEGRADED, FAILING], SAMPLE_SERVICE, Swatch, items, BG (+14 more)

### Community 20 - "Session"
Cohesion: 0.08
Nodes (26): login(), signup(), FINAL, ISO_PROD, OPEN, ALL, AUDITOR, created (+18 more)

### Community 21 - "InvitesTab.tsx"
Cohesion: 0.15
Nodes (21): Card(), InvitePage(), NEXT, noSubscribe(), problem(), InviteForm(), INVITES, InvitesTab() (+13 more)

### Community 22 - "controlhub-actions/src/lib.rs"
Cohesion: 0.07
Nodes (27): ActionSpec, CATALOG, DECISION_WINDOW_SECS, evaluate(), EXECUTION_WINDOW_SECS, MAX_PLANS_PER_HOUR, number(), ParamSpec (+19 more)

### Community 23 - "controlhub-api/src/auth.rs"
Cohesion: 0.10
Nodes (24): all_stored(), API_TOKEN_PREFIX, api_tokens_are_bound_to_one_environment_without_revealing_others(), api_tokens_are_capped_to_read_permissions_of_their_owner(), authorize_as(), Authorized, cookie_values_are_matched_by_exact_name(), Credential (+16 more)

### Community 24 - "roles.rs"
Cohesion: 0.16
Nodes (26): authorize_project(), ProjectAccess, from_stored(), audit(), catalog(), create_role(), delete_role(), deny() (+18 more)

### Community 25 - "web/package.json"
Cohesion: 0.05
Nodes (42): dependencies, @controlhub/api-client, @controlhub/web-engine, next, pixi.js, react, react-dom, @tanstack/react-query (+34 more)

### Community 26 - "generated/index.ts"
Cohesion: 0.10
Nodes (21): ColorNode, Batch, EngineConfig, EntityKindDto, EventItem, EventMarker, HealthStatusDto, LogItem (+13 more)

### Community 27 - "client.rs"
Cohesion: 0.10
Nodes (13): api_get_sends_the_bearer_and_encodes_query_values(), api_json_maps_statuses_without_leaking_the_token(), api_token(), API_TOKEN_ENV, check_url(), Client, ingest_token(), INGEST_TOKEN_ENV (+5 more)

### Community 28 - "signup.rs"
Cohesion: 0.08
Nodes (18): check_new_password(), clean_name(), DEFAULT_ENVIRONMENT, DEFAULT_PROJECT, MAX_NAME_CHARS, MAX_PASSWORD_BYTES, MIN_PASSWORD_CHARS, names_default_and_are_bounded() (+10 more)

### Community 29 - "State"
Cohesion: 0.12
Nodes (29): State, .ALL, Approved, Cancelled, Expired, Failed, Proposed, Rejected (+21 more)

### Community 30 - "Permission"
Cohesion: 0.07
Nodes (20): API_TOKEN_PERMISSIONS, Permission, ActionsApprove, ActionsPropose, ActionsView, .ALL, .MEMBERS_MANAGE, MembersManage (+12 more)

### Community 31 - "controlhub-api/src/main.rs"
Cohesion: 0.06
Nodes (44): account_mutations(), account_project_and_invite_mutations_need_a_session_and_never_accept_api_tokens(), admin_mutations(), admin_routes_need_a_session_and_never_accept_api_tokens(), allowed_origins_are_split_and_trimmed(), api_config(), api_tokens_can_never_mutate(), bearer_values_that_are_not_api_tokens_are_401() (+36 more)

### Community 32 - "invites.rs"
Cohesion: 0.13
Nodes (23): accept(), AcceptedInvite, audit(), create(), CreatedInvite, CreateInviteRequest, DEFAULT_EXPIRY_DAYS, deny() (+15 more)

### Community 33 - "events.rs"
Cohesion: 0.12
Nodes (30): classify_key(), attribute_limits(), base(), convert(), each_validation_rule_rejects_with_a_reason(), EventBatchIn, EventIn, exactly_max_events_is_allowed() (+22 more)

### Community 34 - "controlhub-processor/src/main.rs"
Cohesion: 0.09
Nodes (19): T, Args, CONSUMER, Ctx, dead_letter_payload(), IDLE_TOUCH_INTERVAL, MAX_DEAD_LETTER_PAYLOAD, Metrics (+11 more)

### Community 35 - "Engine"
Cohesion: 0.09
Nodes (9): EdgeLookup, Existing, Missing, NoEdge, Unresolved, Engine, SeekOutcome, Applied (+1 more)

### Community 36 - "controlhub-ingest/src/auth.rs"
Cohesion: 0.12
Nodes (20): Authenticator, cache_is_bounded(), cache_is_keyed_by_hash_not_plaintext(), CACHE_TTL, credential(), CredentialStore, Fake, Pg (+12 more)

### Community 37 - "session.rs"
Cohesion: 0.13
Nodes (20): attrs(), cookie(), csrf_cookie_is_readable_by_js_but_still_strict(), dev_mode_only_drops_secure(), DUMMY_HASH, DUMMY_HASH_FOR_TEST, EnvironmentDto, login() (+12 more)

### Community 38 - "grpc.rs"
Cohesion: 0.10
Nodes (14): auth_failures_map_to_grpc_codes(), auth_header(), broker_failures_are_unavailable_never_ok(), invalid_spans_are_reported_as_partial_success(), kv(), log(), logs(), logs_are_accepted_and_partially_rejected() (+6 more)

### Community 39 - "telemetry.rs"
Cohesion: 0.07
Nodes (33): bad_request(), completeness_from(), completeness_reflects_processing_state(), Cursor, EXTERNAL_CLIENTS_NAME, external_services_and_clients_are_marked_as_outside_the_project(), log_dto(), log_pages_and_dtos() (+25 more)

### Community 40 - "PgStore"
Cohesion: 0.08
Nodes (18): AccountRow, ApiTokenPrincipal, AuditRow, email_domain_patterns(), EnvironmentRow, IngestCredential, InviteLookup, InviteRow (+10 more)

### Community 41 - "ch.rs"
Cohesion: 0.08
Nodes (31): build_topology(), merge_services(), ns(), services_merge_spans_and_logs(), trace_pages_report_more_and_point_the_cursor_at_the_last_item(), trace_row(), CallEdgeRow, ChConfig (+23 more)

### Community 42 - "IntegrationGuide.tsx"
Cohesion: 0.24
Nodes (16): IntegrationGuide(), noSubscribe(), SERVICES, Snippet(), Step(), TOKENS, collectorConfig(), curlCheck() (+8 more)

### Community 43 - "layout.rs"
Cohesion: 0.07
Nodes (46): client_parent_gives_four_distinct_times(), consumer_hop_ends_when_processing_ends(), error_flags(), extreme_times_saturate(), Hop, HOP_CLAMPED, HOP_DIMMED, HOP_ERROR (+38 more)

### Community 44 - "mcp.test.mjs"
Cohesion: 0.13
Nodes (8): bindgenVersion(), bin, sdkPath, tokenIds, DEMO, root, bin, McpClient

### Community 45 - "sanitize.rs"
Cohesion: 0.09
Nodes (30): arrays_and_bytes_are_bounded(), attribute_count_limit_counts_every_dropped_attribute(), attrs_after(), bodies_and_statements_are_dropped_and_counted(), deep_nesting_is_cut_off(), every_span_attribute_location_is_sanitized(), FRAGMENTS, key() (+22 more)

### Community 46 - "plan.rs"
Cohesion: 0.17
Nodes (22): ALLOWED_FILES, apply(), args(), build(), diff(), env_example_step(), fixture(), gitignore_step() (+14 more)

### Community 47 - "PixiMap"
Cohesion: 0.14
Nodes (7): Legend(), MapView(), AnimClock, line(), MapCallbacks, PixiMap, Frame

### Community 48 - "model.rs"
Cohesion: 0.06
Nodes (25): ACTION_AUDIT, CONFIG_CHANGED, DEPLOYMENT_COMPLETED, DEPLOYMENT_STARTED, MAX_ATTRIBUTE_KEY_LEN, MAX_ATTRIBUTE_VALUE_LEN, MAX_ATTRIBUTES_PER_ITEM, MAX_EVENTS_PER_BATCH (+17 more)

### Community 49 - "Timeline"
Cohesion: 0.13
Nodes (10): checkpoints_are_prefix_states(), Delta, empty_timeline(), growing_past_twice_the_budget_is_reported(), latency_sums_wrap_but_differences_stay_exact(), naive(), out_of_range_indices_are_ignored(), span_deltas_update_the_right_counters() (+2 more)

### Community 50 - "secrets.rs"
Cohesion: 0.11
Nodes (19): only_well_formed_invite_secrets_are_looked_up(), StorageError, ClickHouse, Migrate, Other, constant_time_eq_handles_lengths_and_single_bit_differences(), generate_password(), generate_token() (+11 more)

### Community 51 - "authorize"
Cohesion: 0.28
Nodes (15): authorize(), coverage(), decode_cursor(), events(), external_clients_id(), limit(), logs(), parse_ns() (+7 more)

### Community 52 - "bootstrap.rs"
Cohesion: 0.11
Nodes (15): admin_pg(), CH_MIGRATIONS, configured_owner(), DEMO_SCOPE, errors_never_contain_the_password(), Owner, owner_email_is_normalised_and_the_password_kept_verbatim(), rotate_ingest_token() (+7 more)

### Community 53 - "redaction.rs"
Cohesion: 0.07
Nodes (14): AttributeDecision, Drop, Keep, Redact, case_mix(), DROPPED_KEYS, find_scheme(), QUERY_KEYS (+6 more)

### Community 54 - "ChStore"
Cohesion: 0.20
Nodes (3): bind_scope(), ChStore, EventRow

### Community 55 - "SecurityPage.tsx"
Cohesion: 0.17
Nodes (20): Security(), LoginForm(), submit(), LoginPage(), errorText(), RegisterForm(), submit(), RegisterPage() (+12 more)

### Community 56 - "engine.worker.ts"
Cohesion: 0.08
Nodes (23): Pending, RpcRequest, FakeWorker, fetchText(), flush(), handle(), liveConnect(), LiveState (+15 more)

### Community 57 - "pipeline.rs"
Cohesion: 0.15
Nodes (10): Accepted, AcceptError, Auth, Publish, AppState, batch_ids_are_unique(), broker_errors_are_propagated_and_counted(), envelope_scope_comes_from_the_credential() (+2 more)

### Community 58 - "String"
Cohesion: 0.22
Nodes (22): Completeness, CoverageResponse, encode_cursor(), EventDto, EventListResponse, EventParams, log_page(), LogDto (+14 more)

### Community 59 - "doctor.rs"
Cohesion: 0.21
Nodes (19): a_missing_project_directory_is_a_failed_check_not_an_error(), API_SECRET, assert_no_secrets(), check(), conn(), healthy_setup_passes_without_printing_tokens(), INGEST_SECRET, malformed_token_variables_fail_the_doctor() (+11 more)

### Community 60 - "edgeStyleStore.ts"
Cohesion: 0.18
Nodes (14): EDGE_STYLE_KEY, parseEdgeStyle(), setEdgeStyle(), useEdgeStyle(), usePersistedSize(), listeners, memory, readPersisted() (+6 more)

### Community 61 - "tracks.rs"
Cohesion: 0.13
Nodes (16): bucket_and_clip_limits_are_clamped(), depth_counts_same_service_ancestors_and_is_bounded(), Engine, hidden_and_muted_services_leave_logs_and_events_out(), is_request(), markers_are_capped_sorted_and_counted(), MAX_CLIP_CAP, MAX_DEPTH (+8 more)

### Community 62 - "reducer.rs"
Cohesion: 0.09
Nodes (24): arb_deltas(), arb_kind(), budget(), DeltaKind, Event, Log, SpanEnd, SpanStart (+16 more)

### Community 63 - "SettingsPage.tsx"
Cohesion: 0.13
Nodes (31): Settings(), AdminRow(), assignableRole(), CatalogEntry, defaultInviteRole(), groupCatalog(), memberAfter(), MemberGrant (+23 more)

### Community 64 - ".new"
Cohesion: 0.12
Nodes (9): HealthStatus, Degraded, Failing, Healthy, Unknown, classify(), EdgeRec, NodeRec (+1 more)

### Community 65 - "controlhub-runtime/src/lib.rs"
Cohesion: 0.13
Nodes (15): main(), main(), main(), backoff(), backoff_stays_within_jittered_cap(), connect_broker(), connect_broker_gives_up_after_deadline_with_context(), ensure_topic() (+7 more)

### Community 66 - "verify.rs"
Cohesion: 0.17
Nodes (16): Conn, a_rejected_api_token_stops_before_sending_anything(), a_visible_event_and_app_traffic_pass(), an_event_that_never_becomes_visible_fails(), Fake, ingestion_must_acknowledge_with_broker_durability(), missing_app_traffic_only_fails_when_required(), OK (+8 more)

### Community 67 - "ApiError"
Cohesion: 0.15
Nodes (16): ApiError, BadRequest, Conflict, Forbidden, Gone, Internal, NotFound, PolicyDenied (+8 more)

### Community 68 - "draw.ts"
Cohesion: 0.13
Nodes (28): actionColor(), bars(), bucketGeometry(), C, clips(), draw(), DrawInput, drawRuler() (+20 more)

### Community 69 - "ActionPlanRow"
Cohesion: 0.17
Nodes (11): ActionPlanRow, audit_tx(), NewActionPlan, PgStore, PLAN_COLUMNS, PLAN_FROM, row(), system_audit() (+3 more)

### Community 70 - "routes/actions.rs"
Cohesion: 0.12
Nodes (13): ActionCatalogResponse, ActionPlanDetail, ActionSpecDto, denial_messages_list_only_failed_checks(), denied_message(), ParamSpecDto, plan_dto_uses_catalog_titles_and_marks_unknown_types(), PolicyCheckDto (+5 more)

### Community 71 - "discover.rs"
Cohesion: 0.14
Nodes (13): discover(), discovers_node_project_without_reading_env_values(), Discovery, Ecosystem, Node, Unsupported, MAX_MANIFEST_BYTES, names() (+5 more)

### Community 72 - "String"
Cohesion: 0.28
Nodes (15): ActionPlanDto, approve(), bad_request(), cancel(), catalog(), clean_comment(), DecisionRequest, detail() (+7 more)

### Community 73 - "context.rs"
Cohesion: 0.13
Nodes (15): now_ns(), build(), build_propagates_api_errors(), build_queries_the_scoped_api_and_resolves_edge_names(), clean(), list(), MAX_ITEMS, MAX_STRING (+7 more)

### Community 74 - "ref_node_assert"
Cohesion: 0.17
Nodes (12): fault, withTarget(), captureServer(), cleanEnv(), freePort(), runNode(), src(), startNode() (+4 more)

### Community 75 - "ActionsPage.tsx"
Cohesion: 0.18
Nodes (18): Actions(), ActionsPage(), BASE, Json(), message(), OPEN_STATES, Plan, PlanDetail() (+10 more)

### Community 76 - "vitest"
Cohesion: 0.07
Nodes (29): AppLayout(), dynamic, metadata, Tokens(), Providers(), BASE, Centered(), Created (+21 more)

### Community 77 - "SystemPermission"
Cohesion: 0.10
Nodes (16): .TEXT, .TEXT, RuleText, SystemPermission, .ALL, AuditView, .MEMBERS_MANAGE, ProjectsManage (+8 more)

### Community 78 - "ids.rs"
Cohesion: 0.10
Nodes (7): IdError, Empty, InvalidChars, InvalidHex, TooLong, MAX_ID_LEN, validate_token()

### Community 79 - "WebEngine"
Cohesion: 0.21
Nodes (3): set(), to_js(), WebEngine

### Community 80 - "common.cjs"
Cohesion: 0.18
Nodes (15): { sendJson, work, serve }, { sendJson, callJson, work, serve }, http, logger(), LOOPBACK, readJson(), sendJson(), serve() (+7 more)

### Community 81 - "integration/lib.mjs"
Cohesion: 0.09
Nodes (38): waitFinal(), iso, [rootId, extCallId, intCallId, lateCallId, backId], run, sendTrace(), topology(), traceId, event() (+30 more)

### Community 82 - "doctor.mjs"
Cohesion: 0.14
Nodes (10): bindgen, compose, docker, infraEnv, ports, results, rustc, servicePorts (+2 more)

### Community 83 - "test_fixtures.rs"
Cohesion: 0.11
Nodes (10): SystemBuiltin, Admin, .ALL, Auditor, SuperAdmin, CREDENTIALS_FILE, ISOLATION_TOKEN, SYSTEM_USERS (+2 more)

### Community 84 - "pg.rs"
Cohesion: 0.06
Nodes (25): AdminCounts, COUNT_SUPER_ADMINS, PROJECT_COLUMNS, ROLE_ORDER, SUPER_ADMIN, SYSTEM_GRANT, USER_COLUMNS, an_empty_role_prefix_is_refused_before_touching_the_database() (+17 more)

### Community 85 - "controlhub-executor/src/main.rs"
Cohesion: 0.14
Nodes (16): Args, Executor, INGEST_SETTLE, invalid_scope_fails_before_policy_or_effect(), LEASE_SECS, now_ns(), Outcome, Failed (+8 more)

### Community 86 - "live.rs"
Cohesion: 0.08
Nodes (15): EVENT_LOOKBACK_NS, HEARTBEAT, INITIAL_OVERLAP_NS, MAX_QUERY_FAILURES, MAX_ROWS_PER_KIND, POLL, run(), send() (+7 more)

### Community 87 - "controlhub-engine-core/src/lib.rs"
Cohesion: 0.15
Nodes (13): EDGE_STRIDE, event_delta(), FLAG_DECLARED, FLAG_DIMMED, FLAG_EXTERNAL, FLAG_HIGHLIGHT, FLAG_INFERRED, FLAG_PROBLEM (+5 more)

### Community 88 - "sdk-node/package.json"
Cohesion: 0.11
Nodes (18): description, exports, ./register, main, name, private, scripts, test (+10 more)

### Community 89 - "proxy.ts"
Cohesion: 0.18
Nodes (10): nextConfig, securityHeaders, baselineCsp, policy(), strictCsp(), load(), config, isPublicAppPath() (+2 more)

### Community 90 - "testing.rs"
Cohesion: 0.14
Nodes (4): get(), Harness, TOKEN, TRACES_ONLY

### Community 91 - "system_permissions.rs"
Cohesion: 0.13
Nodes (7): account_changes_follow_the_same_reach_as_grants(), an_auditor_changes_nothing(), arb_set(), check_account_change(), only_super_admins_touch_super_admins_and_one_always_remains(), set(), target()

### Community 92 - "controlhub-cli/src/main.rs"
Cohesion: 0.19
Nodes (9): apply_needs_an_explicit_yes(), Cli, context_minutes_are_bounded(), discover_and_plan_run_without_the_network(), main(), Outcome, parse(), run() (+1 more)

### Community 93 - "Event"
Cohesion: 0.12
Nodes (12): ALL_EVENTS, allowed(), Event, Approve, Cancel, Claim, Expire, Fail (+4 more)

### Community 94 - ".from_request_parts"
Cohesion: 0.18
Nodes (7): bearer(), check_csrf(), check_origin(), cookie_value(), forbidden(), is_mutation(), constant_time_eq()

### Community 95 - "TimelineBar.tsx"
Cohesion: 0.14
Nodes (20): Props, Props, RulerInput, View, Props, MODE_STYLE, PRESETS, Props (+12 more)

### Community 96 - "controlhub-web-engine/src/lib.rs"
Cohesion: 0.14
Nodes (7): BATCH, batches_from_the_worker_accumulate_and_dedupe(), config_parsing(), js_err(), parse_config(), status_encoding_matches_the_typescript_wrapper(), TS_INDEX

### Community 97 - "mod.rs"
Cohesion: 0.21
Nodes (6): audit(), audit_rows_map_to_rfc3339_dtos(), AuditEntryDto, health(), HealthResponse, ready()

### Community 98 - "nonempty"
Cohesion: 0.23
Nodes (15): event_dto(), event_dtos_keep_u64_sequences_as_strings(), event_item(), event_row(), log_item(), nonempty(), only_root_server_spans_come_from_external_clients(), span_item() (+7 more)

### Community 99 - "pixi-map.ts"
Cohesion: 0.11
Nodes (25): animTimeMs(), hopPhase, CARD_BOX, drawKindIcon(), LINE_STYLE, near(), NodeText, ROUTE_BOX (+17 more)

### Community 100 - "index.js"
Cohesion: 0.16
Nodes (14): GLOBAL_KEY, hasForeignOtelProvider(), isIncoming(), isServerResponse(), OTEL_API_KEY, pathOf(), SEVERITY, startControlHub() (+6 more)

### Community 101 - "dev.mjs"
Cohesion: 0.21
Nodes (12): apiEnv, FLAGS, parseDevArgs(), bin(), binDir, exe(), procs, services (+4 more)

### Community 102 - "WorkspaceHost.tsx"
Cohesion: 0.42
Nodes (8): APP_PAGES, decode(), keptWorkspace(), parseWorkspacePath(), sameRoute(), demo, WorkspaceRoute, WorkspaceHost()

### Community 103 - "LoginThrottle"
Cohesion: 0.20
Nodes (4): login_throttle_blocks_after_max_failures_per_key(), LoginThrottle, .MAX_FAILURES, .WINDOW

### Community 104 - "logs"
Cohesion: 0.26
Nodes (8): log(), log_bodies_are_truncated_masked_and_structured_bodies_redacted(), log_budget_rejects_the_overflow(), log_validation(), logs(), redacted_value(), string_of(), sv()

### Community 105 - "demo-system/package.json"
Cohesion: 0.12
Nodes (15): dependencies, @controlhub/sdk-node, pg, description, name, private, scripts, fault (+7 more)

### Community 106 - "api-client/package.json"
Cohesion: 0.12
Nodes (15): dependencies, openapi-fetch, devDependencies, openapi-typescript, typescript, exports, typescript, name (+7 more)

### Community 107 - "compilerOptions"
Cohesion: 0.12
Nodes (15): compilerOptions, exactOptionalPropertyTypes, forceConsistentCasingInFileNames, isolatedModules, lib, module, moduleResolution, noEmit (+7 more)

### Community 108 - ".apply_batch"
Cohesion: 0.23
Nodes (6): EngineError, InvalidInput, ItemKey, Event, Log, Span

### Community 109 - "EventsClient"
Cohesion: 0.23
Nodes (5): EventsClient, isValidHex(), nowUnixNanos(), sleep(), client()

### Community 110 - "@playwright/test"
Cohesion: 0.19
Nodes (5): @playwright/test, PUBLIC, login(), user(), PUBLIC_PAGES

### Community 111 - "helpers.ts"
Cohesion: 0.24
Nodes (12): Creds, fixed64(), fixtures, ingestLog(), ingestSpans(), keyValue(), len(), local (+4 more)

### Community 112 - "Prose"
Cohesion: 0.23
Nodes (9): DocsPage(), metadata, FeaturesPage(), metadata, GettingStartedPage(), metadata, IntegrationsPage(), metadata (+1 more)

### Community 113 - "external.ts"
Cohesion: 0.21
Nodes (12): dashes(), dashedRect(), externalExplanation(), isExternalKind(), nodeSubLabel(), ALL, colorAt(), cssColor() (+4 more)

### Community 114 - "precise-start.test.js"
Cohesion: 0.25
Nodes (7): ClockOffset, hrToNs(), MAX_TRUSTED_OFFSET_NS, nsToHr(), perfToEpochNs(), PreciseStartSpanProcessor, refineStart()

### Community 116 - "test_http.rs"
Cohesion: 0.22
Nodes (4): read_request(), Seen, serve(), session_json()

### Community 117 - "skew.rs"
Cohesion: 0.17
Nodes (3): Engine, fit(), MAX_WALK

### Community 118 - "Scope"
Cohesion: 0.19
Nodes (8): CreatedAccount, new_account_scope(), NewAccount, signed_up_scopes_are_valid_random_and_never_collide_with_seeds(), TokenKind, Api, Ingest, TokenRow

### Community 119 - "Permissions and project roles — design"
Cohesion: 0.15
Nodes (12): Anti-escalation rules (server-side), API, Built-in roles, Custom templates, Goal, Member role, Out of scope (later parts), Permission catalog (+4 more)

### Community 120 - "web-engine/package.json"
Cohesion: 0.15
Nodes (12): description, devDependencies, typescript, exports, ./pkg, typescript, name, private (+4 more)

### Community 121 - "e2e/package.json"
Cohesion: 0.15
Nodes (12): devDependencies, @playwright/test, @types/node, typescript, @types/node, typescript, name, private (+4 more)

### Community 122 - "api-surface.test.mjs"
Cohesion: 0.11
Nodes (13): FAKE_TRACE, mutations, operations, PUBLIC, RANGE_ROUTES, scopedGets, spec, API (+5 more)

### Community 123 - "Action"
Cohesion: 0.18
Nodes (8): Action, DemoFaultsClear, DemoFaultsInject, DemoRestartDatabase, DemoService, Gateway, Orders, stored_action()

### Community 124 - "Command"
Cohesion: 0.20
Nodes (9): Command, Apply, Context, Discover, Doctor, Mcp, Plan, Verify (+1 more)

### Community 125 - "EntityKind"
Cohesion: 0.17
Nodes (11): EntityKind, Cache, Database, Deployment, ExternalClient, ExternalDependency, InfrastructureResource, Instance (+3 more)

### Community 126 - "check_member_change"
Cohesion: 0.23
Nodes (14): Actor, Actor<P>, check_invite(), check_member_change(), check_template_change(), Grant, Individual, Remove (+6 more)

### Community 128 - "dependencies"
Cohesion: 0.17
Nodes (12): dependencies, @opentelemetry/api, @opentelemetry/api-logs, @opentelemetry/exporter-logs-otlp-proto, @opentelemetry/exporter-trace-otlp-proto, @opentelemetry/instrumentation, @opentelemetry/instrumentation-http, @opentelemetry/instrumentation-pg (+4 more)

### Community 129 - "collectErrors"
Cohesion: 0.17
Nodes (3): collectErrors(), ownerAccount(), owner

### Community 130 - "File map"
Cohesion: 0.15
Nodes (13): ProjectDto, key(), File map, Global Constraints, Permissions and project roles — implementation plan, Task 1: Permission model (pure), Task 2: Schema and storage, Task 3: Authorization by permission (+5 more)

### Community 131 - "compilerOptions"
Cohesion: 0.18
Nodes (10): compilerOptions, allowJs, incremental, jsx, paths, plugins, exclude, extends (+2 more)

### Community 132 - "bench.rs"
Cohesion: 0.27
Nodes (4): main(), Rng, SERVICES, synthetic()

### Community 133 - "Builtin"
Cohesion: 0.12
Nodes (13): arb_op(), arb_set(), Builtin, Admin, .ALL, Operator, Owner, builtin_roles_match_the_spec() (+5 more)

### Community 134 - "state.rs"
Cohesion: 0.29
Nodes (4): ApiConfig, AppState, SignupPolicy, test_state()

### Community 135 - "ADR-0011: Read-only API tokens and the WebSocket live channel"
Cohesion: 0.18
Nodes (6): ADR-0011: Read-only API tokens and the WebSocket live channel, Alternatives rejected, API tokens (`cha_…`), Ingestion token rotation, Live channel (`GET …/live`, WebSocket), ControlHubLogger

### Community 136 - "package.json"
Cohesion: 0.18
Nodes (10): engines, node, name, packageManager, private, scripts, build, lint (+2 more)

### Community 137 - "controlhub-domain"
Cohesion: 0.36
Nodes (11): controlhub-actions, controlhub-api, controlhub-domain, controlhub-engine-core, controlhub-executor, controlhub-ingest, controlhub-processor, controlhub-protocol (+3 more)

### Community 138 - "Splitter.tsx"
Cohesion: 0.42
Nodes (7): clampSize(), SPLITTER_KEY_STEP, SplitterAxis, splitterDragValue(), splitterKeyValue(), Props, Splitter()

### Community 139 - "openapi.rs"
Cohesion: 0.22
Nodes (3): ApiDoc, BearerAuth, spec_json()

### Community 140 - "AuthError"
Cohesion: 0.31
Nodes (6): AuthError, Invalid, Missing, SignalNotAllowed, Unavailable, bearer()

### Community 141 - "super"
Cohesion: 0.11
Nodes (7): Error, deserialize(), NANOS_PER_MILLI, NANOS_PER_SEC, O, serialize(), main()

### Community 142 - "adr/README.md"
Cohesion: 0.20
Nodes (7): ADR-0006: Local ports, Consequences, Decision, ADR-0010: Pixi renderer lives in apps/web, Decision, Reason, Architecture decision records

### Community 143 - "README.md"
Cohesion: 0.22
Nodes (4): Data flow, Delivery guarantees, Timeline and time machine, Topology

### Community 144 - "Commands"
Cohesion: 0.20
Nodes (10): Commands, context, `controlhub` CLI, Credentials, discover, mcp, plan / apply, Tested (+2 more)

### Community 145 - "orders.cjs"
Cohesion: 0.22
Nodes (8): callJson(), faults, http, log, { logger, sendJson, createFaults, callJson, listen }, { Pool }, port, server

### Community 146 - "gateway.cjs"
Cohesion: 0.24
Nodes (9): getJson(), listen(), faults, http, log, { logger, sendJson, createFaults, getJson, callJson, listen }, port, postJson() (+1 more)

### Community 147 - "Onboarding.tsx"
Cohesion: 0.26
Nodes (12): OnboardingPage(), useSeenServices(), InviteStep(), Onboarding(), Stepper(), Env, nextStep(), onboardingTarget() (+4 more)

### Community 148 - "Map routing, service focus and replay range — design"
Cohesion: 0.22
Nodes (8): Decisions (asked and answered), Design, Engine (`controlhub-engine-core`), Goals, Map routing, service focus and replay range — design, Renderer (`apps/web/src/renderer`), Tests, Web UI

### Community 149 - "index.d.ts"
Cohesion: 0.22
Nodes (5): Attributes, ControlHubHandle, ControlHubOptions, EventFields, EventsStats

### Community 150 - "ControlHub — tasks"
Cohesion: 0.22
Nodes (9): ControlHub — tasks, Cross-cutting, Stage 0 — repository, Stage 1 — end-to-end flow, Stage 2 — viewer, Stage 3 — timeline, Stage 4 — coding-agent integration, Stage 5 — controlled actions (+1 more)

### Community 152 - "grid.ts"
Cohesion: 0.46
Nodes (6): GRID_MAJOR_EVERY, GRID_MAX_PX, GRID_MIN_PX, GridLine, gridLines(), gridStep()

### Community 153 - "Scope"
Cohesion: 0.21
Nodes (6): derived_ids_have_stable_shape(), EntityId, same_name_different_scope_never_collides(), Scope, scoped_hash(), ServiceId

### Community 154 - "Security model (current implementation)"
Cohesion: 0.25
Nodes (8): Authentication, Controlled actions (ADR-0012), Headers, Multi-tenancy, Not done yet, Secrets and environments, Security model (current implementation), Sensitive data

### Community 155 - "Layered, zoomable timeline — design"
Cohesion: 0.25
Nodes (7): Architecture, Goal, Honesty and limits, Interaction, Layered, zoomable timeline — design, Testing, Tracks (top to bottom)

### Community 156 - "ref_node_http"
Cohesion: 0.25
Nodes (5): port, server, http, port, server

### Community 157 - "config.test.js"
Cohesion: 0.25
Nodes (6): getControlHub(), ENV_KEYS, GLOBAL_KEY, OTEL_API_KEY, handle, server

### Community 158 - "invites.spec.ts"
Cohesion: 0.25
Nodes (3): secret(), pageAlert(), openInviteLinks()

### Community 159 - "web-engine-contract.test.ts"
Cohesion: 0.17
Nodes (6): FLAGS, HOP_FLAGS, done(), hook(), lockPath, logPath

### Community 160 - "live"
Cohesion: 0.24
Nodes (4): live(), LiveParams, require_origin(), start_cursor()

### Community 162 - "Local development and operations"
Cohesion: 0.29
Nodes (6): Generated local state (`.controlhub/`, gitignored), Health and metrics, Lifecycle, Local development and operations, Ports (all bound to 127.0.0.1), Troubleshooting

### Community 163 - "node-http-app/package.json"
Cohesion: 0.29
Nodes (6): description, name, private, scripts, start, version

### Community 164 - "cull.ts"
Cohesion: 0.47
Nodes (3): CULL_OVERSCAN, cullRect(), WorldRect

### Community 165 - "common.test.mjs"
Cohesion: 0.47
Nodes (5): createFaults(), { createFaults, readJson, sendJson, callJson, getJson, serve }, faultServer(), listen(), require

### Community 166 - "e2e/tsconfig.json"
Cohesion: 0.33
Nodes (5): compilerOptions, types, extends, include, ../../tsconfig.base.json

### Community 167 - "AGENTS.md — working in the ControlHub repo"
Cohesion: 0.40
Nodes (4): AGENTS.md — working in the ControlHub repo, Commands, Layout, Rules

### Community 169 - "sanitize_traces"
Cohesion: 0.35
Nodes (11): mask_bearer(), sanitize_attribute(), sanitize_value(), truncate(), redacts_and_rejects(), sanitize_any(), sanitize_attrs(), sanitize_logs() (+3 more)

### Community 170 - "ADR-0001: Monorepo layout"
Cohesion: 0.40
Nodes (4): ADR-0001: Monorepo layout, Consequences, Context, Decision

### Community 171 - "ADR-0002: Kafka client — rskafka"
Cohesion: 0.40
Nodes (4): ADR-0002: Kafka client — rskafka, Consequences, Context, Decision

### Community 172 - "ADR-0003: Consumer offsets in PostgreSQL, at-least-once"
Cohesion: 0.40
Nodes (4): ADR-0003: Consumer offsets in PostgreSQL, at-least-once, Consequences, Context, Decision

### Community 173 - "ADR-0004: ClickHouse dedup via ReplacingMergeTree"
Cohesion: 0.40
Nodes (4): ADR-0004: ClickHouse dedup via ReplacingMergeTree, Consequences, Context, Decision

### Community 174 - "ADR-0005: WASM engine in a Worker, content-hashed artifact"
Cohesion: 0.40
Nodes (4): ADR-0005: WASM engine in a Worker, content-hashed artifact, Consequences, Context, Decision

### Community 175 - "ADR-0009: The Worker fetches JSON and hands raw text to WASM"
Cohesion: 0.40
Nodes (4): ADR-0009: The Worker fetches JSON and hands raw text to WASM, Consequences, Context, Decision

### Community 176 - "ADR-0012: Controlled actions — plans, policy, four-eyes approval, separate executor"
Cohesion: 0.40
Nodes (4): ADR-0012: Controlled actions — plans, policy, four-eyes approval, separate executor, Alternatives rejected, Consequences, Decision

### Community 177 - "ADR-0013: Read-only MCP server as a CLI subcommand over the REST API"
Cohesion: 0.40
Nodes (4): ADR-0013: Read-only MCP server as a CLI subcommand over the REST API, Alternatives rejected, Consequences, Decision

### Community 178 - "ADR-0014: OTLP metrics are not ingested (yet); service metrics come from spans"
Cohesion: 0.40
Nodes (4): ADR-0014: OTLP metrics are not ingested (yet); service metrics come from spans, Decision, When to revisit, Why

### Community 179 - "Invite links, personal account and onboarding — design"
Cohesion: 0.40
Nodes (4): Invite links, Invite links, personal account and onboarding — design, Onboarding, Personal account (`/app/account`)

### Community 180 - "fault.mjs"
Cohesion: 0.40
Nodes (3): opts, [service, ...rest], targets

### Community 181 - "loadgen.mjs"
Cohesion: 0.40
Nodes (3): counts, requestedRps, rps

### Community 182 - "ControlHub"
Cohesion: 0.40
Nodes (5): Checks, ControlHub, Documentation, Quick start (local demo), Requirements

### Community 183 - "StepState"
Cohesion: 0.50
Nodes (4): StepState, AlreadyApplied, Applied, Pending

### Community 184 - "reference.rs"
Cohesion: 0.30
Nodes (6): is_entry(), metrics_at(), only_entry_spans_count_and_duplicates_collapse(), RefNodeMetrics, span(), window_is_open_at_the_start_and_closed_at_the_end()

### Community 186 - "ADR-0007: Browser authentication"
Cohesion: 0.50
Nodes (3): ADR-0007: Browser authentication, Alternatives rejected, Decision

### Community 187 - "ADR-0008: TypeScript 5.9"
Cohesion: 0.50
Nodes (3): ADR-0008: TypeScript 5.9, Decision, Reason

### Community 188 - "api-client/tsconfig.json"
Cohesion: 0.50
Nodes (3): extends, include, ../../tsconfig.base.json

### Community 189 - "web-engine/tsconfig.json"
Cohesion: 0.50
Nodes (3): extends, include, ../../tsconfig.base.json

### Community 191 - "AI integration"
Cohesion: 0.67
Nodes (3): AI integration, Rules for any AI client, Surfaces

### Community 198 - "MemberChange"
Cohesion: 0.31
Nodes (5): Postgres, MemberChange, Done, LastOwner, NotFound

### Community 199 - "Signal"
Cohesion: 0.33
Nodes (6): Signal, Events, Logs, Metrics, Traces, signal_name()

## Knowledge Gaps
- **865 isolated node(s):** `lockPath`, `logPath`, `securityHeaders`, `nextConfig`, `name` (+860 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 1445 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **14 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `vitest` connect `vitest` to `TimelineTracksView.tsx`, `unwrap`, `Workspace.tsx`, `Splitter.tsx`, `useWorkspaceEngine.ts`, `edge-path.ts`, `map-key.ts`, `Onboarding.tsx`, `InvitesTab.tsx`, `grid.ts`, `web/package.json`, `web-engine-contract.test.ts`, `cull.ts`, `IntegrationGuide.tsx`, `SecurityPage.tsx`, `engine.worker.ts`, `edgeStyleStore.ts`, `SettingsPage.tsx`, `draw.ts`, `ActionsPage.tsx`, `proxy.ts`, `TimelineBar.tsx`, `pixi-map.ts`, `WorkspaceHost.tsx`, `external.ts`, `seo.test.ts`?**
  _High betweenness centrality (0.024) - this node is a cross-community bridge._
- **Why does `State` connect `State` to `invites.rs`, `live`, `mod.rs`, `http.rs`, `CurrentUser`, `session.rs`, `String`, `tokens.rs`, `authorize`, `controlhub-actions/src/lib.rs`, `roles.rs`, `signup.rs`, `Event`?**
  _High betweenness centrality (0.015) - this node is a cross-community bridge._
- **Why does `PgStore` connect `PgStore` to `.pool`, `pg.rs`, `MemberChange`, `Scope`?**
  _High betweenness centrality (0.014) - this node is a cross-community bridge._
- **What connects `lockPath`, `logPath`, `securityHeaders` to the rest of the system?**
  _865 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `.pool` be split into smaller, more focused modules?**
  _Cohesion score 0.1427304964539007 - nodes in this community are weakly interconnected._
- **Should `http.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.054343434343434346 - nodes in this community are weakly interconnected._
- **Should `TimelineTracksView.tsx` be split into smaller, more focused modules?**
  _Cohesion score 0.11839323467230443 - nodes in this community are weakly interconnected._