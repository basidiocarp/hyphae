// SQLite interop uses i64/usize conversions throughout via rusqlite's integer type.
// Suppress systemic pedantic lints that require broad mechanical changes across
// hundreds of SQLite query functions. These are tracked as a follow-up.
#![allow(
    // SQLite integer ↔ Rust integer casts (rusqlite uses i64 for all SQL integers).
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_lossless,
    // Doc section requirements — hundreds of SQLite query functions need # Errors.
    clippy::missing_errors_doc,
    // Doc formatting — widespread in query function comments.
    clippy::doc_markdown,
    // Closure style — common in iterator chains over rusqlite rows.
    clippy::redundant_closure_for_method_calls,
    // String construction — common pattern in query builders.
    clippy::manual_string_new,
    // Format string inlining — widely spread.
    clippy::uninlined_format_args,
    // map().unwrap_or() — idiomatic in query result transformations.
    clippy::map_unwrap_or,
    // Float equality in tests (clamped boundary checks).
    clippy::float_cmp,
    // Wildcard match arms — in enums with many variants.
    clippy::match_wildcard_for_single_variants,
    // Similar binding names — common in SQLite row destructuring.
    clippy::similar_names,
    // must_use — accessor methods in query result structs.
    clippy::must_use_candidate,
    // Default() vs Default::default() — stylistic.
    clippy::default_trait_access,
    // Unnecessary pass by value — in query function signatures.
    clippy::needless_pass_by_value,
    // Items after statements — test helper functions.
    clippy::items_after_statements,
    // unused self — trait stubs.
    clippy::unused_self,
    // Large test functions — integration tests with complex setup.
    clippy::too_many_lines,
    // match arms with same body — reviewed and intentional.
    clippy::match_same_arms,
    // if_not_else — stylistic.
    clippy::if_not_else,
    // Vec::default() patterns.
    clippy::vec_init_then_push,
    // Assigning clones.
    clippy::assigning_clones,
)]

pub mod memoir_community;
pub mod schema;
mod store;

pub use hyphae_core::ChunkStore;
pub use hyphae_core::{Artifact, ArtifactType};
pub use hyphae_core::{SearchOrder, TopicMemoryOrder};
pub use store::SHARED_PROJECT;
pub use store::SqliteStore;
pub use store::UnifiedSearchResult;
pub use store::audit::{AuditEntry, AuditOperation};
pub use store::dispatch_search;
pub use store::evaluation::{
    EvaluationWindow, RecallEffectivenessRow, RecallEffectivenessWindow, collect_evaluation_window,
    collect_recall_effectiveness_window,
};
pub use store::export::{
    ArchiveFilter, ArchiveIdentity, ArchiveMemoirConceptRecord, ArchiveMemoirLinkRecord,
    ArchiveMemoirRecord, ArchiveMemoryRecord, ArchiveSessionRecord, HyphaeArchive,
};
pub use store::insights::{
    HyphaeActivitySnapshot, HyphaeAnalytics, LessonCategory, LessonRecord, RecentMemoryActivity,
};
pub use store::passive::{
    CompactSummaryArtifact, PassiveContextBundle, PassiveMemoryItem, ProjectUnderstandingBundle,
    ProjectUnderstandingConcept,
};
pub use store::session::{Session, SessionTimelineEvent, SessionTimelineRecord};
pub mod context {
    pub use crate::store::context::*;
}
