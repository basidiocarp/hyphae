use anyhow::Result;
use hyphae_core::MemoryStore;
use hyphae_store::SqliteStore;

pub(crate) fn cmd_prune(
    store: &SqliteStore,
    threshold: Option<f32>,
    dry_run: bool,
) -> Result<()> {
    if dry_run {
        let expired_count = store.count_expired()?;
        println!("Expired ephemeral memories: {expired_count}");

        if let Some(t) = threshold {
            let low_weight_count = store.count_low_weight(t)?;
            println!("Low-weight memories (below {t}): {low_weight_count}");
            println!(
                "Total would be pruned: {}",
                expired_count + low_weight_count
            );
        }

        println!("(dry run — nothing was deleted)");
        return Ok(());
    }

    let expired = store.prune_expired()?;
    println!("Pruned {expired} expired ephemeral memories");

    if let Some(t) = threshold {
        let low_weight = store.prune(t)?;
        println!("Pruned {low_weight} low-weight memories (below {t})");
    }

    Ok(())
}
