pub mod cache;
pub mod catalog;
pub mod catalog_chains;
pub mod catalog_edges;
pub mod catalog_gaps;
pub mod catalog_projects;
#[cfg(feature = "neural-search")]
mod catalog_entities;
#[cfg(feature = "neural-search")]
pub mod alerts;
pub mod embed_policy;
#[cfg(feature = "neural-search")]
pub mod entities;
#[cfg(feature = "semantic-search")]
pub mod evidence;
pub mod explain;
pub mod extraction;
pub mod failure_log;
pub mod file_reader;
#[cfg(feature = "semantic-search")]
pub mod lexical;
pub mod line_quality;
pub mod matching;
pub mod models;
pub mod native_index;
pub mod near;
#[cfg(feature = "nl-find")]
pub mod nl_find;
#[cfg(feature = "nl-find")]
pub mod nl_rules;
#[cfg(feature = "neural-search")]
pub mod neural_search;
#[cfg(feature = "ocr")]
pub mod ocr;
pub mod orchestrator;
pub mod ordering;
mod pdf_decrypt;
#[cfg(feature = "pdf-yolow")]
mod pdf_yolow;
pub mod regex_literals;
#[cfg(feature = "semantic-search")]
mod pdf_page_map;
#[cfg(feature = "neural-search")]
pub mod relevance;
pub mod report;
#[cfg(feature = "semantic-search")]
pub mod ranking;
#[cfg(feature = "semantic-search")]
mod semantic_cache;
#[cfg(feature = "semantic-search")]
pub mod semantic_index;
pub mod synonyms;
pub mod watch;
pub mod textfold;
