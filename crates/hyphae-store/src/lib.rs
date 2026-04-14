mod schema;
mod store;

pub use hyphae_core::ChunkStore;
pub use store::SHARED_PROJECT;
pub use store::SqliteStore;
pub use store::UnifiedSearchResult;
pub use store::audit::{AuditEntry, AuditOperation};
pub use store::evaluation::{
    EvaluationWindow, RecallEffectivenessRow, RecallEffectivenessWindow, collect_evaluation_window,
    collect_recall_effectiveness_window,
};
pub use store::insights::{
    HyphaeActivitySnapshot, HyphaeAnalytics, LessonCategory, LessonRecord, RecentMemoryActivity,
};
pub use store::passive::{
    CompactSummaryArtifact, PassiveContextBundle, PassiveMemoryItem, ProjectUnderstandingBundle,
    ProjectUnderstandingConcept,
};
pub use store::session::{Session, SessionTimelineEvent, SessionTimelineRecord};
pub use store::{SearchOrder, TopicMemoryOrder};
pub mod context {
    pub use crate::store::context::*;
}
