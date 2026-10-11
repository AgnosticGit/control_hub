#![allow(clippy::expect_used)]
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use karbird_domain::redaction::{classify_key, mask_bearer, sanitize_attribute};
use karbird_domain::{EnvironmentId, ProjectId, Scope, ServiceId, TenantId};

fn domain(c: &mut Criterion) {
    let value = "https://api.example/orders/123?access_token=secret&customer=123#summary";
    c.bench_function("domain/redact_url_attribute", |b| {
        b.iter(|| sanitize_attribute(black_box("http.url"), black_box(value)))
    });
    c.bench_function("domain/classify_header", |b| b.iter(|| classify_key(black_box("http.request.header.x-api-key"))));
    c.bench_function("domain/classify_common_attribute", |b| b.iter(|| classify_key(black_box("http.request.method"))));
    c.bench_function("domain/mask_bearer_plain_value", |b| {
        b.iter(|| mask_bearer(black_box("GET /api/orders/:id completed in 12 ms")))
    });
    c.bench_function("domain/mask_bearer_secret_value", |b| {
        b.iter(|| mask_bearer(black_box("request failed with Bearer abc.def-123")))
    });
    let scope = Scope {
        tenant_id: TenantId::parse("tenant-main").expect("fixture"),
        project_id: ProjectId::parse("project-checkout").expect("fixture"),
        environment_id: EnvironmentId::parse("production").expect("fixture"),
    };
    c.bench_function("domain/derive_scoped_service_id", |b| {
        b.iter(|| ServiceId::derive(black_box(&scope), black_box("commerce"), black_box("orders")))
    });
}

criterion_group!(benches, domain);
criterion_main!(benches);
