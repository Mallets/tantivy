use std::io::{self, Write};

use tantivy::collector::{Count, TopDocs};
use tantivy::query::{BooleanQuery, Occur, PhraseQuery, Query, QueryParser, TermQuery};
use tantivy::schema::*;
use tantivy::tokenizer::TokenizerManager;
use tantivy::{Index, IndexWriter, Opstamp, ReloadPolicy, TantivyDocument};
use tempfile::TempDir;

const B: &str = "\x1b[1m";
const G: &str = "\x1b[1;32m";
const Y: &str = "\x1b[1;33m";
const C: &str = "\x1b[1;36m";
const D: &str = "\x1b[2m";
const R: &str = "\x1b[0m";

const LINES: &[&str] = &[
    r#"{"service":"web","level":"error","message":"Connection refused to host db-primary port 5432","latency":120}"#,
    r#"{"service":"web","level":"warn","message":"Connection refused to host db-replica port 5432","latency":95}"#,
    r#"{"service":"web","level":"error","message":"Connection refused to host db-backup port 5432","latency":110}"#,
    r#"{"service":"web","level":"error","message":"Connection refused to host cache-primary port 6379","latency":85}"#,
    r#"{"service":"web","level":"error","message":"Connection refused to host cache-replica port 6379","latency":90}"#,
    r#"{"service":"web","level":"warn","message":"Connection refused to host queue-primary port 5672","latency":200}"#,
    r#"{"service":"web","level":"error","message":"Connection refused to host queue-replica port 5672","latency":210}"#,
    r#"{"service":"web","level":"error","message":"Connection refused to host db-read1 port 5432","latency":115}"#,
    r#"{"service":"web","level":"error","message":"Connection refused to host db-read2 port 5432","latency":125}"#,
    r#"{"service":"web","level":"error","message":"Connection refused to host search-primary port 9200","latency":150}"#,
    r#"{"service":"web","level":"error","message":"Connection refused to host search-replica port 9200","latency":155}"#,
    r#"{"service":"web","level":"error","message":"Connection refused to host monitor-primary port 8086","latency":80}"#,
    r#"{"service":"web","level":"info","message":"Health check passed","latency":1}"#,
    r#"{"service":"web","level":"warn","message":"Connection refused to host db-standby port 5432","latency":100}"#,
    r#"{"service":"web","level":"warn","message":"Connection refused to host db-analytics port 5432","latency":105}"#,
    r#"{"service":"api","level":"info","message":"GET /orders completed in 45ms","http":{"method":"GET","code":200},"latency":45}"#,
    r#"{"service":"api","level":"info","message":"GET /users completed in 12ms","http":{"method":"GET","code":200},"latency":12}"#,
    r#"{"service":"api","level":"error","message":"POST /checkout failed in 3002ms","http":{"method":"POST","code":500},"latency":3002}"#,
    r#"{"service":"api","level":"error","message":"POST /checkout failed in 5002ms","http":{"method":"POST","code":500},"latency":5002}"#,
    r#"{"service":"api","level":"error","message":"POST /checkout failed in 1002ms","http":{"method":"POST","code":500},"latency":1002}"#,
    r#"{"service":"api","level":"warn","message":"GET /api/v2 rate limited","http":{"method":"GET","code":429},"latency":0}"#,
    r#"{"service":"api","level":"info","message":"GET /products completed in 33ms","http":{"method":"GET","code":200},"latency":33}"#,
    r#"{"service":"api","level":"info","message":"GET /accounts completed in 27ms","http":{"method":"GET","code":200},"latency":27}"#,
    r#"{"service":"api","level":"info","message":"GET /settings completed in 8ms","http":{"method":"GET","code":200},"latency":8}"#,
    r#"{"service":"api","level":"info","message":"GET /dashboard completed in 51ms","http":{"method":"GET","code":200},"latency":51}"#,
    r#"{"service":"api","level":"info","message":"GET /profile completed in 14ms","http":{"method":"GET","code":200},"latency":14}"#,
];

fn wait(label: &str) {
    print!("\n{C}>>> Press ENTER to: {label}{R} ");
    io::stdout().flush().unwrap();
    let mut buf = String::new();
    io::stdin().read_line(&mut buf).unwrap();
}

fn sep() {
    println!("{D}{}{R}", "─".repeat(80));
}

fn main() -> tantivy::Result<()> {
    let input_size: usize = LINES.iter().map(|l| l.len() + 1).sum();

    println!("{Y}╔══════════════════════════════════════════════════════════════════╗{R}");
    println!("{Y}║       Tantivy Ingestion & Query Pipeline — Step-by-Step Tour     ║{R}");
    println!("{Y}╚══════════════════════════════════════════════════════════════════╝{R}");
    println!();
    println!(
        "  Input: {B}{} JSON log lines{R} ({} bytes)",
        LINES.len(),
        input_size
    );
    println!();
    println!("  {B}Glossary:{R}");
    println!("  ┌────────────────────┬─────────────────────────────────────────────────┐");
    println!("  │ {C}Schema{R}             │ Defines the fields of every document — their    │");
    println!("  │                    │ names, types, and how they are indexed/stored.  │");
    println!("  ├────────────────────┼─────────────────────────────────────────────────┤");
    println!("  │ {C}Field{R}              │ A named column in the schema (e.g. \"service\").  │");
    println!("  │                    │ Typed: text, u64, i64, f64, json, etc.          │");
    println!("  ├────────────────────┼─────────────────────────────────────────────────┤");
    println!("  │ {C}Segment{R}            │ An immutable chunk of the index. New segments   │");
    println!("  │                    │ are created on commit; merged in background.    │");
    println!("  ├────────────────────┼─────────────────────────────────────────────────┤");
    println!("  │ {C}Term{R}               │ A (field, value) pair. e.g. (level, \"error\").   │");
    println!("  │                    │ The unit of lookup in the inverted index.       │");
    println!("  ├────────────────────┼─────────────────────────────────────────────────┤");
    println!("  │ {C}Posting List{R}       │ Sorted list of DocIds containing a term.        │");
    println!("  │                    │ Optionally stores term frequency and positions. │");
    println!("  ├────────────────────┼─────────────────────────────────────────────────┤");
    println!("  │ {C}Term Dictionary{R}    │ FST mapping Term → TermOrdinal → TermInfo.      │");
    println!("  │                    │ Enables O(term_len) lookups.                    │");
    println!("  ├────────────────────┼─────────────────────────────────────────────────┤");
    println!("  │ {C}Doc Store{R}          │ Row-oriented compressed storage for STORED      │");
    println!("  │                    │ fields. Used to reconstruct search results.     │");
    println!("  ├────────────────────┼─────────────────────────────────────────────────┤");
    println!("  │ {C}Fast Field{R}         │ Column-oriented storage for numeric fields.     │");
    println!("  │                    │ Bitpacked. O(1) random access by DocId.         │");
    println!("  ├────────────────────┼─────────────────────────────────────────────────┤");
    println!("  │ {C}BM25{R}               │ Relevance scoring: considers term frequency,    │");
    println!("  │                    │ inverse document frequency, and field length.   │");
    println!("  └────────────────────┴─────────────────────────────────────────────────┘");
    println!();
    println!("  {B}Acronyms:{R}");
    println!("  ┌────────────────────┬─────────────────────────────────────────────────┐");
    println!("  │ {C}IDF{R}                │ Inverse Document Frequency — measures how rare  │");
    println!("  │                    │ a term is across all documents.                 │");
    println!("  ├────────────────────┼─────────────────────────────────────────────────┤");
    println!("  │ {C}TF{R}                 │ Term Frequency — how often a term appears in a  │");
    println!("  │                    │ single document.                                │");
    println!("  ├────────────────────┼─────────────────────────────────────────────────┤");
    println!("  │ {C}FST{R}                │ Finite State Transducer — compact automaton used│");
    println!("  │                    │ as the term dictionary for O(key_len) lookups.  │");
    println!("  ├────────────────────┼─────────────────────────────────────────────────┤");
    println!("  │ {C}BM25{R}               │ Best Matching 25 — ranking function combining   │");
    println!("  │                    │ TF, IDF, and document length normalization.     │");
    println!("  ├────────────────────┼─────────────────────────────────────────────────┤");
    println!("  │ {C}DocId{R}              │ Document Identifier — segment-local u32 assigned│");
    println!("  │                    │ sequentially (0, 1, 2, ...) during indexing.    │");
    println!("  └────────────────────┴─────────────────────────────────────────────────┘");
    println!();
    println!("  {B}Pipeline:{R}");
    println!();
    println!("  {Y}INGESTION{R}");
    println!("     ┌────────────┐     ┌────────────┐     ┌────────────┐     ┌────────────┐");
    println!("     │ {C}1. Schema{R}  │────▶│ {C}2. Index{R}   │────▶│ {C}3. Add{R}     │────▶│ {C}4. Commit{R}  │");
    println!("     │  {D}Define{R}    │     │  {D}Create{R}    │     │  {D}Documents{R} │     │  {D}& Flush{R}   │");
    println!("     └────────────┘     └────────────┘     └────────────┘     └────────────┘");
    println!();
    println!("  {Y}QUERY{R}");
    println!("     ┌────────────┐     ┌────────────┐     ┌────────────┐     ┌────────────┐");
    println!("     │ {C}5. Reader{R}  │────▶│ {C}6. Parse{R}   │────▶│ {C}7. Search{R}  │────▶│ {C}8. Collect{R} │");
    println!("     │  {D}& Search{R}  │     │  {D}Query{R}     │     │  {D}Execute{R}   │     │  {D}Results{R}   │");
    println!("     └────────────┘     └────────────┘     └────────────┘     └────────────┘");

    // ═══════════════════════════════════════════════════════════════════════════
    // PART I: INGESTION PATH
    // ═══════════════════════════════════════════════════════════════════════════

    // ── STEP 1: Schema Definition ──────────────────────────────────────────────
    wait("Step 1 — Define the schema");
    sep();
    println!("{B}STEP 1: Schema Definition{R}");
    println!();
    println!("  The schema declares every field before indexing begins.");
    println!("  For each field we choose:");
    println!("    • {C}Type{R}:    text, u64, i64, f64, json, ip, date, bool, bytes");
    println!("    • {C}Indexed{R}: tokenized and stored in the inverted index (TEXT/STRING)");
    println!("    • {C}Stored{R}:  kept in the doc store for retrieval (STORED)");
    println!("    • {C}Fast{R}:    column-oriented for filtering/aggregation (FAST)");
    println!();

    println!("  {B}Sample documents being ingested:{R}");
    println!();
    let samples: &[usize] = &[0, 15];
    for &i in samples {
        let json: serde_json::Value = serde_json::from_str(LINES[i]).unwrap();
        println!(
            "  {G}[{i:>2}]{R} {}",
            serde_json::to_string_pretty(&json)
                .unwrap()
                .lines()
                .enumerate()
                .map(|(li, l)| if li == 0 {
                    l.to_string()
                } else {
                    format!("       {l}")
                })
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    println!();

    let mut schema_builder = Schema::builder();

    let service = schema_builder.add_text_field("service", STRING | STORED);
    let level = schema_builder.add_text_field("level", STRING | STORED);
    let message = schema_builder.add_text_field("message", TEXT | STORED);
    let latency = schema_builder.add_u64_field(
        "latency",
        NumericOptions::default()
            .set_indexed()
            .set_fast()
            .set_stored(),
    );
    let http_method = schema_builder.add_text_field("http_method", STRING | STORED);
    let http_code = schema_builder.add_u64_field(
        "http_code",
        NumericOptions::default()
            .set_indexed()
            .set_fast()
            .set_stored(),
    );

    let schema = schema_builder.build();

    println!("  {B}Schema fields:{R}");
    println!("  ┌──────────────┬──────────┬─────────┬────────┬──────┐");
    println!("  │ Name         │ Type     │ Indexed │ Stored │ Fast │");
    println!("  ├──────────────┼──────────┼─────────┼────────┼──────┤");
    println!("  │ service      │ text     │ STRING  │  ✓     │      │");
    println!("  │ level        │ text     │ STRING  │  ✓     │      │");
    println!("  │ message      │ text     │ TEXT    │  ✓     │      │");
    println!("  │ latency      │ u64      │  ✓      │  ✓     │  ✓   │");
    println!("  │ http_method  │ text     │ STRING  │  ✓     │      │");
    println!("  │ http_code    │ u64      │  ✓      │  ✓     │  ✓   │");
    println!("  └──────────────┴──────────┴─────────┴────────┴──────┘");
    println!();
    println!("  {D}STRING = not tokenized (exact match). TEXT = tokenized (full-text).{R}");
    println!("  {D}STORED = retrievable from doc store. FAST = column-oriented numeric.{R}");
    println!();
    println!("  {B}Key distinction:{R}");
    println!("    {C}STRING{R}: \"Connection refused\" is one token → exact match only");
    println!("    {C}TEXT{R}:   \"Connection refused\" → [\"connection\", \"refused\"] → each searchable");

    // ── STEP 2: Index Creation ─────────────────────────────────────────────────
    wait("Step 2 — Create the index");
    sep();
    println!("{B}STEP 2: Index Creation{R}");
    println!();
    println!("  An Index is a collection of immutable Segments, stored in a directory.");
    println!("  We create it from the schema. A {C}meta.json{R} file tracks the segment list.");
    println!();

    let index_path = TempDir::new().expect("failed to create temp dir");
    let index = Index::create_in_dir(&index_path, schema.clone())?;

    println!("  Index directory: {G}{}{R}", index_path.path().display());
    println!();
    println!("  {B}What was created:{R}");
    println!("    • {C}meta.json{R} — schema + empty segment list");
    println!("    • No segments yet — documents haven't been added");
    println!();
    println!("  {B}Segment anatomy (after commit):{R}");
    println!("    Each segment contains these files ({D}UUID.ext{R}):");
    println!("    ┌──────────────────┬────────────────────────────────────────────┐");
    println!("    │ {C}.term{R}            │ Term dictionary (FST: term → TermInfo)     │");
    println!("    │ {C}.pos{R}             │ Positions (for phrase queries)             │");
    println!("    │ {C}.idx{R}             │ Postings (sorted DocId lists per term)     │");
    println!("    │ {C}.store{R}           │ Doc store (compressed row storage)         │");
    println!("    │ {C}.fast{R}            │ Fast fields (bitpacked column storage)     │");
    println!("    │ {C}.fieldnorm{R}       │ Field norms (token count per doc per field)│");
    println!("    └──────────────────┴────────────────────────────────────────────┘");

    // ── STEP 3: Adding Documents ───────────────────────────────────────────────
    wait("Step 3 — Add documents via IndexWriter");
    sep();
    println!("{B}STEP 3: Adding Documents{R}");
    println!();
    println!("  The IndexWriter is the single entry point for mutations.");
    println!("  It owns a memory arena (we allocate 50MB) where terms and postings");
    println!("  accumulate. Internally it spawns indexing threads, each building a");
    println!("  SegmentWriter with:");
    println!("    • {C}PerFieldPostingsWriter{R} — inverted index in memory");
    println!("    • {C}FastFieldsWriter{R}       — columnar data");
    println!("    • {C}FieldNormsWriter{R}       — token counts per field");
    println!("    • {C}StoreWriter{R}            — compressed doc storage");
    println!();
    println!("  {B}For each document, the SegmentWriter:{R}");
    println!("     1. Assigns a {C}DocId{R} (0, 1, 2, ...) within the segment");
    println!("     2. For TEXT fields: runs the tokenizer, records each token");
    println!("        in the postings writer with (term, doc_id, position)");
    println!("     3. For STRING fields: records the entire value as one term");
    println!("     4. For FAST fields: appends value to the columnar writer");
    println!("     5. For STORED fields: serializes the doc for the store");
    println!();

    let mut index_writer: IndexWriter = index.writer(50_000_000)?;

    let mut doc_count = 0u32;
    for line in LINES {
        let json: serde_json::Value = serde_json::from_str(line).unwrap();
        let obj = json.as_object().unwrap();

        let svc = obj["service"].as_str().unwrap();
        let lvl = obj["level"].as_str().unwrap();
        let msg = obj["message"].as_str().unwrap();
        let lat = obj["latency"].as_u64().unwrap();

        let mut tantivy_doc = TantivyDocument::default();
        tantivy_doc.add_text(service, svc);
        tantivy_doc.add_text(level, lvl);
        tantivy_doc.add_text(message, msg);
        tantivy_doc.add_u64(latency, lat);

        if let Some(http) = obj.get("http") {
            if let Some(m) = http.get("method") {
                tantivy_doc.add_text(http_method, m.as_str().unwrap());
            }
            if let Some(c) = http.get("code") {
                tantivy_doc.add_u64(http_code, c.as_u64().unwrap());
            }
        }

        index_writer.add_document(tantivy_doc)?;
        doc_count += 1;
    }

    println!("  {G}Added {doc_count} documents to the IndexWriter.{R}");
    println!();
    println!("  {B}Tokenization example (message field, TEXT):{R}");
    println!("  Input:  {D}\"Connection refused to host db-primary port 5432\"{R}");
    print!("  Tokens: ");

    let tokenizer_manager = TokenizerManager::default();
    let mut tokenizer = tokenizer_manager.get("default").unwrap();
    let mut stream = tokenizer.token_stream("Connection refused to host db-primary port 5432");
    let mut tokens = Vec::new();
    while let Some(tok) = stream.next() {
        tokens.push(tok.text.clone());
    }
    for (i, t) in tokens.iter().enumerate() {
        if i > 0 {
            print!(", ");
        }
        print!("{C}\"{t}\"{R}");
    }
    println!();
    println!();
    println!("  {D}Note: \"db-primary\" → \"db\" + \"primary\" (hyphen splits tokens){R}");
    println!("  {D}All tokens lowercased. Positions tracked for phrase queries.{R}");
    println!();
    println!("  {B}What happens in memory:{R}");
    println!("    ┌─────────────────────────────────────────────────────────┐");
    println!("    │  {C}Indexing Hash Table (stacker){R}                          │");
    println!("    │                                                         │");
    println!("    │  term \"connection\" → doc_ids: [0,1,2,3,4,5,6,7,8,9,..]  │");
    println!("    │  term \"refused\"    → doc_ids: [0,1,2,3,4,5,6,7,8,9,..]  │");
    println!("    │  term \"error\"      → doc_ids: [0,2,3,4,6,7,8,9,10,11]   │");
    println!("    │  term \"web\"        → doc_ids: [0,1,2,3,4,5,6,7,8,...]   │");
    println!("    │  ...                                                    │");
    println!("    │                                                         │");
    println!("    │  Each entry also tracks: term frequency, positions      │");
    println!("    └─────────────────────────────────────────────────────────┘");

    // ── STEP 4: Commit ─────────────────────────────────────────────────────────
    wait("Step 4 — Commit: flush to disk as an immutable segment");
    sep();
    println!("{B}STEP 4: Commit & Segment Serialization{R}");
    println!();
    println!("  Commit flushes the in-memory data into an immutable on-disk segment.");
    println!("  This is {B}the point where data becomes searchable{R}.");
    println!();
    println!("  {B}Serialization steps (per SegmentWriter):{R}");
    println!("     1. {C}Postings{R}: terms are sorted, posting lists are delta-encoded");
    println!("        and bitpacked in blocks of 128 docs");
    println!("     2. {C}Term Dictionary{R}: an FST maps each term to its TermInfo");
    println!("        (doc_freq, postings byte range, positions byte range)");
    println!("     3. {C}Positions{R}: per-term positions written for phrase queries");
    println!("     4. {C}Fast Fields{R}: columnar values bitpacked (min + offset)");
    println!("     5. {C}Field Norms{R}: token count per doc, used by BM25");
    println!("     6. {C}Doc Store{R}: documents block-compressed (LZ4/zstd)");
    println!("     7. {C}meta.json{R} updated atomically with new segment list");
    println!();

    let commit_opstamp: Opstamp = index_writer.commit()?;

    println!("  {G}Commit successful!{R} opstamp={commit_opstamp}");
    println!();

    let segment_ids: Vec<_> = index.searchable_segment_ids()?;
    println!("  Segments on disk: {}", segment_ids.len());
    for sid in &segment_ids {
        println!("    • {C}{sid:?}{R}");
    }
    println!();

    let dir_entries: Vec<_> = std::fs::read_dir(index_path.path())?
        .filter_map(|e| e.ok())
        .collect();
    let mut file_list: Vec<_> = dir_entries
        .iter()
        .map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            (name, size)
        })
        .collect();
    file_list.sort();

    println!("  {B}Files on disk:{R}");
    let mut total_size = 0u64;
    for (name, size) in &file_list {
        total_size += size;
        let ext = name.rsplit('.').next().unwrap_or("");
        let desc = match ext {
            "term" => "term dictionary (FST)",
            "idx" => "postings (inverted index)",
            "pos" => "positions (phrase queries)",
            "store" => "doc store (compressed)",
            "fast" => "fast fields (columnar)",
            "fieldnorm" => "field norms (BM25)",
            "json" => "metadata",
            _ => "",
        };
        println!("    {name:<50} {:>8}  {D}{desc}{R}", format_bytes(*size));
    }
    println!("    {D}Total: {}{R}", format_bytes(total_size));
    println!();
    println!("  {B}Inverted index layout:{R}");
    println!("    ┌───────────────────┐    ┌───────────────────┐    ┌────────────────┐");
    println!("    │ {C}Term Dictionary{R}   │    │ {C}Posting Lists{R}     │    │ {C}Positions{R}      │");
    println!("    │ (.term file)      │    │ (.idx file)       │    │ (.pos file)    │");
    println!("    │                   │    │                   │    │                │");
    println!("    │ Term → TermInfo   │──▶ │ [DocId, DocId,..] │──▶ │ [pos, pos,...] │");
    println!("    │  (FST + SSTable)  │    │  delta + bitpack  │    │  (for phrases) │");
    println!("    └───────────────────┘    └───────────────────┘    └────────────────┘");
    println!();
    println!("  {B}TermInfo = {{doc_freq, postings_range, positions_range}}{R}");
    println!("  The term dict maps: Term → TermOrdinal (via FST) → TermInfo");

    // ═══════════════════════════════════════════════════════════════════════════
    // PART II: QUERY PATH
    // ═══════════════════════════════════════════════════════════════════════════

    // ── STEP 5: Reader & Searcher ──────────────────────────────────────────────
    wait("Step 5 — Create Reader and Searcher (immutable snapshot)");
    sep();
    println!("{B}STEP 5: Reader & Searcher{R}");
    println!();
    println!("  A {C}Reader{R} maintains a pool of {C}Searchers{R}. Each Searcher holds a");
    println!("  snapshot of the index — a list of SegmentReaders. The snapshot is");
    println!("  immutable: commits and merges don't affect an open Searcher.");
    println!();

    let reader = index
        .reader_builder()
        .reload_policy(ReloadPolicy::OnCommitWithDelay)
        .try_into()?;
    let searcher = reader.searcher();

    println!("  Searcher acquired:");
    println!("    Segment readers: {}", searcher.segment_readers().len());
    for (i, sr) in searcher.segment_readers().iter().enumerate() {
        println!(
            "    [{i}] max_doc={}, num_docs={} (alive), segment_id={:?}",
            sr.max_doc(),
            sr.num_docs(),
            sr.segment_id()
        );
    }
    println!();
    println!("  {B}SegmentReader provides access to:{R}");
    println!("    • {C}InvertedIndexReader{R} — term dict + postings per field");
    println!("    • {C}FastFieldReaders{R}    — columnar numeric access");
    println!("    • {C}StoreReader{R}         — document retrieval");
    println!("    • {C}FieldNormReader{R}     — token counts for BM25");

    // ── STEP 6: Query Parsing ──────────────────────────────────────────────────
    wait("Step 6 — Parse a user query into a Query AST");
    sep();
    println!("{B}STEP 6: Query Parsing{R}");
    println!();
    println!("  The {C}QueryParser{R} turns a text string into a {C}Query{R} object.");
    println!("  It uses the query-grammar crate (parser combinators) to build an AST.");
    println!();
    println!("  {B}Query types available:{R}");
    println!("    • {C}TermQuery{R}      — single term lookup");
    println!("    • {C}BooleanQuery{R}   — AND / OR / NOT composition");
    println!("    • {C}PhraseQuery{R}    — exact phrase match using positions");
    println!("    • {C}RangeQuery{R}     — numeric/text ranges");
    println!("    • {C}FuzzyQuery{R}     — Levenshtein distance match");
    println!("    • {C}RegexQuery{R}     — regex over term dictionary");
    println!("    • {C}AllQuery{R}       — match all documents");
    println!("    • {C}ExistsQuery{R}    — field existence check");
    println!("    • {C}ConstScoreQuery{R}— fixed score wrapper");
    println!();

    let query_parser = QueryParser::for_index(&index, vec![message]);

    println!("  {B}Example: parsing \"connection refused\"{R}");
    let q1 = query_parser.parse_query("connection refused")?;
    println!("    Parsed query: {q1:?}");
    println!();
    println!("  {D}Default: OR between terms. \"connection refused\" matches docs{R}");
    println!("  {D}containing \"connection\" OR \"refused\" (or both, ranked higher).{R}");
    println!();

    println!("  {B}Example: parsing '\"connection refused\"' (phrase query){R}");
    let q2 = query_parser.parse_query("\"connection refused\"")?;
    println!("    Parsed query: {q2:?}");
    println!();
    println!("  {D}Phrase query requires both terms adjacent in order.{R}");

    // ── STEP 7: Search Execution ───────────────────────────────────────────────
    wait("Step 7 — Execute queries and walk through the search path");
    sep();
    println!("{B}STEP 7: Search Execution{R}");
    println!();
    println!("  {B}How a query executes:{R}");
    println!("    1. {C}Query::weight(){R}  — compute IDF stats across all segments");
    println!("    2. {C}Weight::scorer(){R}  — for each segment, look up the term dict,");
    println!("       open the posting list, create a Scorer (DocSet + scoring)");
    println!("    3. {C}Collector::collect(){R} — iterate the scorer, collect results");
    println!();
    println!("  {B}Term → Posting List lookup:{R}");
    println!("    ┌────────────────┐   ┌───────────────┐   ┌─────────────────────┐");
    println!(
        "    │ {C}\"error\"{R}        │──▸│ {C}TermInfo{R}      │──▸│ {C}PostingList{R}         │"
    );
    println!("    │ (level field)  │   │ doc_freq: N   │   │ [doc2,3,4,6,7,9..]  │");
    println!("    │                │   │ postings: ..  │   │ delta+bitpacked     │");
    println!("    └────────────────┘   └───────────────┘   └─────────────────────┘");
    println!();

    // 7a: TermQuery — single term
    println!("  {Y}7a) TermQuery — Find docs with level=\"error\"{R}");
    sep();
    let term_error = Term::from_field_text(level, "error");
    let term_query = TermQuery::new(term_error, IndexRecordOption::WithFreqs);
    let top_docs = searcher.search(&term_query, &TopDocs::with_limit(20).order_by_score())?;
    let count = searcher.search(&term_query, &Count)?;

    println!("    Query: TermQuery(level, \"error\")");
    println!("    Matching docs: {G}{count}{R}");
    println!();
    for (score, doc_address) in &top_docs {
        let doc: TantivyDocument = searcher.doc(*doc_address)?;
        let json = doc.to_json(&schema);
        println!(
            "    score={score:.4} seg={} doc={}",
            doc_address.segment_ord, doc_address.doc_id
        );
        println!("      {D}{json}{R}");
    }

    // 7b: BooleanQuery — AND
    println!();
    println!("  {Y}7b) BooleanQuery AND — level=\"error\" AND service=\"api\"{R}");
    sep();
    let term_api = Term::from_field_text(service, "api");
    let bool_query = BooleanQuery::new(vec![
        (
            Occur::Must,
            Box::new(TermQuery::new(
                Term::from_field_text(level, "error"),
                IndexRecordOption::WithFreqs,
            )) as Box<dyn Query>,
        ),
        (
            Occur::Must,
            Box::new(TermQuery::new(term_api, IndexRecordOption::WithFreqs)) as Box<dyn Query>,
        ),
    ]);
    let results = searcher.search(&bool_query, &TopDocs::with_limit(10).order_by_score())?;
    let count = searcher.search(&bool_query, &Count)?;

    println!("    Matching docs: {G}{count}{R}");
    println!();
    println!("    {D}Boolean AND intersects the two posting lists:{R}");
    println!("    {D}  level:error  → [d0,d2,d3,d4,d6,d7,...]{R}");
    println!("    {D}  service:api  → [d15,d16,d17,d18,...]{R}");
    println!("    {D}  AND result   → intersection of above{R}");
    println!();
    for (score, doc_address) in &results {
        let doc: TantivyDocument = searcher.doc(*doc_address)?;
        println!("    score={score:.4}  {}", doc.to_json(&schema));
    }

    // 7c: PhraseQuery
    println!();
    println!("  {Y}7c) PhraseQuery — \"connection refused\"{R}");
    sep();
    let phrase_query = PhraseQuery::new(vec![
        Term::from_field_text(message, "connection"),
        Term::from_field_text(message, "refused"),
    ]);
    let results = searcher.search(&phrase_query, &TopDocs::with_limit(5).order_by_score())?;
    let count = searcher.search(&phrase_query, &Count)?;

    println!("    Matching docs: {G}{count}{R}");
    println!();
    println!("    {D}Phrase query uses positions to verify adjacency:{R}");
    println!("    {D}  \"connection\" at pos 0,  \"refused\" at pos 1 → match{R}");
    println!("    {D}  Uses the .pos file for each candidate doc from posting intersection{R}");
    println!();
    for (score, doc_address) in results.iter().take(3) {
        let doc: TantivyDocument = searcher.doc(*doc_address)?;
        let json = doc.to_json(&schema);
        println!("    score={score:.4}  {D}{json}{R}");
    }
    if results.len() > 3 {
        println!("    ... and {} more", results.len() - 3);
    }

    // ── STEP 8: Collector & Scoring ────────────────────────────────────────────
    wait("Step 8 — Collectors, BM25 scoring, and advanced queries");
    sep();
    println!("{B}STEP 8: Collectors & Scoring{R}");
    println!();
    println!("  {B}Collectors define what to do with matching docs:{R}");
    println!("    • {C}TopDocs{R}  — keep top-K by score (uses a BinaryHeap)");
    println!("    • {C}Count{R}    — just count matches");
    println!("    • {C}Custom{R}   — implement the Collector trait for any aggregation");
    println!();
    println!("  {B}BM25 Scoring (k1={}, b={}):{R}", 1.2, 0.75);
    println!("    score(D, Q) = Σ IDF(qi) · (tf · (k1+1)) / (tf + k1 · (1 - b + b · |D|/avgdl))");
    println!();
    println!("    Where:");
    println!("      • {C}IDF{R}    = ln(1 + (N - df + 0.5) / (df + 0.5))");
    println!("      • {C}tf{R}     = term frequency in document");
    println!("      • {C}|D|{R}    = document length (from field norms)");
    println!("      • {C}avgdl{R}  = average document length in collection");
    println!();

    // Show explanation for a query
    println!("  {Y}Score explanation for \"checkout\" (message field):{R}");
    let q = query_parser.parse_query("checkout")?;
    let top = searcher.search(&q, &TopDocs::with_limit(1).order_by_score())?;
    if let Some((score, doc_addr)) = top.first() {
        let explanation = q.explain(&searcher, *doc_addr)?;
        println!(
            "    Doc: seg={} doc={} score={score:.4}",
            doc_addr.segment_ord, doc_addr.doc_id
        );
        println!("    {D}{}{R}", explanation.to_pretty_json());
    }

    // 8a: Full-text search with QueryParser
    println!();
    println!("  {Y}8a) Full-text search: \"failed checkout 500\"{R}");
    sep();

    let multi_field_qp = QueryParser::for_index(&index, vec![message, level, service]);
    let q = multi_field_qp.parse_query("failed checkout 500")?;
    let results = searcher.search(&q, &TopDocs::with_limit(5).order_by_score())?;
    println!("    Matching docs: {G}{}{R}", results.len());
    for (score, doc_address) in &results {
        let doc: TantivyDocument = searcher.doc(*doc_address)?;
        println!("    score={score:.4}  {D}{}{R}", doc.to_json(&schema));
    }

    // 8b: Fast field access
    println!();
    println!("  {Y}8b) Fast field access — read latency without doc store{R}");
    sep();
    println!("    Fast fields provide O(1) random access by DocId.");
    println!("    Useful for sorting, filtering, aggregation.");
    println!();

    for segment_reader in searcher.segment_readers() {
        let ff = segment_reader.fast_fields();
        let latency_col = ff.u64("latency").unwrap().first_or_default_col(0);
        let max_doc = segment_reader.max_doc();
        let mut sum = 0u64;
        let mut max_lat = 0u64;
        for doc_id in 0..max_doc {
            let val = latency_col.get_val(doc_id);
            sum += val;
            if val > max_lat {
                max_lat = val;
            }
        }
        let avg = sum as f64 / max_doc as f64;
        println!(
            "    Segment {:?}: {max_doc} docs, avg_latency={avg:.1}, max_latency={max_lat}",
            segment_reader.segment_id()
        );
    }

    // 8c: SegmentReader term dictionary inspection
    println!();
    println!("  {Y}8c) Term dictionary inspection{R}");
    sep();
    println!("    Walking the term dictionary for the \"level\" field:");
    println!();

    for segment_reader in searcher.segment_readers() {
        let inverted_index = segment_reader.inverted_index(level)?;
        let term_dict = inverted_index.terms();

        println!("    ┌──────────────┬──────────┬─────────────────────────┐");
        println!("    │ Term         │ doc_freq │ Postings range          │");
        println!("    ├──────────────┼──────────┼─────────────────────────┤");

        let mut term_stream = term_dict.stream()?;
        while term_stream.advance() {
            let key = term_stream.key();
            let term_str = String::from_utf8_lossy(key);
            let term_info = term_stream.value();
            println!(
                "    │ {:<12} │ {:>8} │ {}..{} ({} bytes)        │",
                term_str,
                term_info.doc_freq,
                term_info.postings_range.start,
                term_info.postings_range.end,
                term_info.postings_range.len()
            );
        }
        println!("    └──────────────┴──────────┴─────────────────────────┘");
    }

    // 8d: Boolean NOT query
    println!();
    println!("  {Y}8d) Boolean NOT — level:error AND NOT service:web{R}");
    sep();
    let bool_not_query = BooleanQuery::new(vec![
        (
            Occur::Must,
            Box::new(TermQuery::new(
                Term::from_field_text(level, "error"),
                IndexRecordOption::WithFreqs,
            )) as Box<dyn Query>,
        ),
        (
            Occur::MustNot,
            Box::new(TermQuery::new(
                Term::from_field_text(service, "web"),
                IndexRecordOption::WithFreqs,
            )) as Box<dyn Query>,
        ),
    ]);
    let results = searcher.search(&bool_not_query, &TopDocs::with_limit(10).order_by_score())?;
    let count = searcher.search(&bool_not_query, &Count)?;
    println!("    Matching docs: {G}{count}{R} (api errors only)");
    for (score, doc_address) in &results {
        let doc: TantivyDocument = searcher.doc(*doc_address)?;
        println!("    score={score:.4}  {D}{}{R}", doc.to_json(&schema));
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // Summary
    // ═══════════════════════════════════════════════════════════════════════════
    println!();
    sep();
    println!("{B}Key Takeaways{R}");
    println!();
    println!("  {B}Ingestion:{R}");
    println!("    1. Schema is fixed at index creation — defines fields and their indexing");
    println!("    2. IndexWriter accumulates docs in memory, one SegmentWriter per thread");
    println!("    3. Commit serializes segments: term dict (FST), postings (bitpacked),");
    println!("       positions, fast fields (columnar), doc store (LZ4/zstd), field norms");
    println!("    4. Segments are immutable — background merges compact them");
    println!();
    println!("  {B}Query:{R}");
    println!("    1. Searcher holds an immutable snapshot of segment readers");
    println!("    2. QueryParser converts text → Query AST (terms, booleans, phrases)");
    println!("    3. For each segment: term dict lookup → posting list → Scorer");
    println!("    4. BM25 scoring uses IDF, term frequency, and field length");
    println!("    5. Collectors aggregate results (TopDocs, Count, custom)");
    println!("    6. Fast fields enable O(1) numeric access for filtering/sorting");
    println!();

    println!("{Y}╔══════════════════════════════════════════════════════════════════╗{R}");
    println!("{Y}║                         Tour complete!                           ║{R}");
    println!("{Y}╚══════════════════════════════════════════════════════════════════╝{R}");

    Ok(())
}

fn format_bytes(n: u64) -> String {
    if n >= 1_048_576 {
        format!("{:.1} MB", n as f64 / 1_048_576.0)
    } else if n >= 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{n} B")
    }
}
