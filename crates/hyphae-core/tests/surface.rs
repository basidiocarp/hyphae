use hyphae_core::{Concept, Importance, Label, Memoir, MemoirStore, Memory, MemoryStore, Relation};

#[test]
fn core_surface_stays_domain_only() {
    let _: Option<&dyn MemoryStore> = None;
    let _: Option<&dyn MemoirStore> = None;

    let memory = Memory::builder(
        "decisions/core".to_string(),
        "Core types stay focused on domain data".to_string(),
        Importance::Medium,
    )
    .keywords(vec!["core".to_string(), "boundary".to_string()])
    .build();

    let memoir = Memoir::new(
        "core-boundary".to_string(),
        "Core crate contract surface".to_string(),
    );
    let concept = Concept::new(
        memoir.id.clone(),
        "core-surface".to_string(),
        "Domain-only concept surface".to_string(),
    );
    let label: Label = "domain:memory".parse().expect("valid label");
    let relation = Relation::RelatedTo;

    assert_eq!(memory.topic, "decisions/core");
    assert_eq!(memoir.name, "core-boundary");
    assert_eq!(concept.name, "core-surface");
    assert_eq!(label.to_string(), "domain:memory");
    assert!(relation.is_symmetric());
}
