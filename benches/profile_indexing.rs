#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

use std::path::PathBuf;

use clap::Parser;
use tantivy::directory::RamDirectory;
use tantivy::merge_policy::NoMergePolicy;
use tantivy::schema::*;
use tantivy::{tokenizer, Index, IndexWriter, TantivyDocument};

const WIKI_EMBEDDED: &str = include_str!("wiki.json");

/// Standalone workload for CPU and memory profiling of tantivy's indexing pipeline.
///
/// Indexes a JSON-lines dataset in a loop using a single-threaded IndexWriter
/// with no merging, isolating pure indexing cost.
#[derive(Parser)]
#[command(name = "profile_indexing")]
struct Args {
    /// Number of indexing iterations to run.
    #[arg(short, long, default_value_t = 100)]
    iterations: usize,

    /// Path to a JSON-lines input file. If omitted, uses the embedded wiki.json (~1000 docs).
    #[arg(short, long)]
    file: Option<PathBuf>,

    /// Writer heap budget in bytes.
    #[arg(short, long, default_value_t = 50_000_000)]
    buffer_size: usize,
}

fn build_index(schema: &Schema, ram: RamDirectory) -> Index {
    let mut index =
        Index::create(ram, schema.clone(), tantivy::IndexSettings::default()).unwrap();
    let ff_tokenizer_manager = tokenizer::TokenizerManager::default();
    ff_tokenizer_manager.register(
        "raw",
        tokenizer::TextAnalyzer::builder(tokenizer::RawTokenizer::default())
            .filter(tokenizer::RemoveLongFilter::limit(255))
            .build(),
    );
    index.set_fast_field_tokenizers(ff_tokenizer_manager);
    index
}

fn main() {
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();

    let args = Args::parse();

    let input = if let Some(path) = &args.file {
        std::fs::read_to_string(path).expect("Failed to read input file")
    } else {
        WIKI_EMBEDDED.to_string()
    };

    let lines: Vec<&str> = input.trim().lines().collect();
    let num_lines = lines.len();

    let mut schema_builder = Schema::builder();
    schema_builder.add_text_field("title", TEXT | STORED);
    schema_builder.add_text_field("body", TEXT);
    schema_builder.add_text_field("url", STRING | STORED);
    let schema = schema_builder.build();

    let ram = RamDirectory::create();
    let index = build_index(&schema, ram.clone());

    eprintln!(
        "Profiling {} iterations x {} documents (buffer {})",
        args.iterations, num_lines, args.buffer_size
    );
    let start = std::time::Instant::now();
    for i in 0..args.iterations {
        let mut writer: IndexWriter =
            index.writer_with_num_threads(1, args.buffer_size).unwrap();
        writer.set_merge_policy(Box::new(NoMergePolicy));

        for line in &lines {
            let doc = TantivyDocument::parse_json(&schema, line).unwrap();
            writer.add_document(doc).unwrap();
        }
        writer.commit().unwrap();

        if args.iterations > 1 {
            eprintln!("  iteration {}/{} done", i + 1, args.iterations);
        }
    }

    let elapsed = start.elapsed();
    let total_docs = args.iterations * num_lines;
    let bytes_total = args.iterations * input.len();
    let index_bytes = ram.total_mem_usage();

    let reader = index.reader().unwrap();
    let searcher = reader.searcher();
    let space = searcher.space_usage().unwrap();

    eprintln!(
        "Done: {} docs in {:.2}s ({:.0} docs/s, {:.2} MB/s)",
        total_docs,
        elapsed.as_secs_f64(),
        total_docs as f64 / elapsed.as_secs_f64(),
        bytes_total as f64 / 1_000_000.0 / elapsed.as_secs_f64(),
    );
    eprintln!("Index size: {:.2} MB ({} bytes)", mb(index_bytes as u64), index_bytes);
    eprintln!("  Segment space (per searcher): {:.2} MB", mb(space.total().get_bytes()));
    for (i, seg) in space.segments().iter().enumerate() {
        eprintln!(
            "  segment {i}: total {:.2} MB  postings {:.2} MB  positions {:.2} MB  \
             store {:.2} MB  fast_fields {:.2} MB  fieldnorms {:.2} MB  termdict {:.2} MB",
            mb(seg.total().get_bytes()),
            mb(seg.postings().total().get_bytes()),
            mb(seg.positions().total().get_bytes()),
            mb(seg.store().total().get_bytes()),
            mb(seg.fast_fields().total().get_bytes()),
            mb(seg.fieldnorms().total().get_bytes()),
            mb(seg.termdict().total().get_bytes()),
        );
    }
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / 1_000_000.0
}
