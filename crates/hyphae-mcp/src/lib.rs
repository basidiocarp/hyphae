#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_lossless,
    clippy::missing_errors_doc,
    clippy::doc_markdown,
    clippy::redundant_closure_for_method_calls,
    clippy::manual_string_new,
    clippy::uninlined_format_args,
    clippy::map_unwrap_or,
    clippy::match_wildcard_for_single_variants,
    clippy::similar_names,
    clippy::must_use_candidate,
    clippy::needless_pass_by_value,
    clippy::items_after_statements,
    clippy::too_many_lines,
    clippy::match_same_arms,
    clippy::if_not_else,
    clippy::manual_let_else,
    clippy::format_collect,
    clippy::unnecessary_wraps,
    clippy::single_match_else,
    clippy::bool_to_int_with_if,
    clippy::string_add,
    clippy::string_add_assign,
    clippy::option_if_let_else,
    clippy::semicolon_if_nothing_returned,
    clippy::useless_conversion,
    clippy::format_push_string
)]

#[cfg(unix)]
mod cap_methods;
pub mod memoir_events;
pub mod memory_protocol;
pub mod protocol;
pub mod server;
#[cfg(unix)]
pub mod socket_server;
mod text;
pub mod tools;

pub use server::run_server;
#[cfg(unix)]
pub use socket_server::run_socket_server;
