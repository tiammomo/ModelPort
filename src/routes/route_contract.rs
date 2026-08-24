use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RouteContract {
    pub(super) domain: &'static str,
    pub(super) path: &'static str,
    pub(super) methods: &'static [&'static str],
}

impl RouteContract {
    pub(super) const fn new(
        domain: &'static str,
        path: &'static str,
        methods: &'static [&'static str],
    ) -> Self {
        Self {
            domain,
            path,
            methods,
        }
    }
}

pub(super) fn all() -> Vec<RouteContract> {
    [
        super::ops::ROUTES,
        super::client_api::ROUTES,
        super::ops_agent::INTERNAL_ROUTES,
        super::admin_auth::ROUTES,
        super::governance_routes::ROUTES,
        super::ops_agent::ADMIN_ROUTES,
        super::admin_providers::ROUTES,
        super::admin_control::ROUTES,
        super::admin_evidence::ROUTES,
        super::admin_identity::ROUTES,
    ]
    .into_iter()
    .flatten()
    .copied()
    .collect()
}

#[test]
fn inventory_has_complete_unique_method_ownership() {
    let contracts = all();
    assert_eq!(contracts.len(), 68, "update the reviewed route inventory");

    let domain_sources = [
        include_str!("ops.rs"),
        include_str!("client_api.rs"),
        include_str!("ops_agent.rs"),
        include_str!("admin_auth.rs"),
        include_str!("governance.rs"),
        include_str!("admin_providers.rs"),
        include_str!("admin_control.rs"),
        include_str!("admin_evidence.rs"),
        include_str!("admin_identity.rs"),
    ];
    let registration_count = domain_sources
        .iter()
        .map(|source| source.matches(".route(").count())
        .sum::<usize>();
    assert_eq!(
        registration_count,
        contracts.len(),
        "every domain registration must have one route contract",
    );

    let root_source = include_str!("../routes.rs");
    let production_root = root_source
        .split("#[cfg(test)]\nmod tests")
        .next()
        .expect("production route module");
    assert!(
        !production_root.contains(".route("),
        "the root router may compose domains but may not own routes",
    );

    let mut owners = BTreeMap::new();
    for contract in contracts {
        assert!(contract.path.starts_with('/'));
        assert!(!contract.domain.is_empty());
        assert!(!contract.methods.is_empty());
        for method in contract.methods {
            let previous = owners.insert((method, contract.path), contract.domain);
            assert!(
                previous.is_none(),
                "{method} {} is owned by both {} and {}",
                contract.path,
                previous.unwrap_or("unknown"),
                contract.domain,
            );
        }
    }
}
