use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::Path;
use regex::Regex;
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use std::time::Instant;
use rayon::prelude::*;
use xxhash_rust::xxh3::xxh3_64;
use std::hint::black_box;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::Arc;

// Global hash cache for avoiding redundant computations
lazy_static::lazy_static! {
    static ref HASH_CACHE: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));
}

// Add lazy_static dependency for global cache
// Note: In production, consider using a more sophisticated cache with LRU eviction

#[derive(Debug, Clone)]
struct PerformanceConfig {
    use_parallel_hashing: bool,
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        PerformanceConfig {
            use_parallel_hashing: true,
        }
    }
}

lazy_static::lazy_static! {
    static ref PERF_CONFIG: Arc<Mutex<PerformanceConfig>> = Arc::new(Mutex::new(PerformanceConfig::default()));
}

fn get_perf_config() -> PerformanceConfig {
    PERF_CONFIG.lock().unwrap().clone()
}


#[derive(Serialize, Deserialize)]
struct LineDiff {
    line_range: String,    // e.g., "L100-L120"
    change_type: String,   // "added", "removed", "modified"
    content_preview: String,
}

#[derive(Serialize, Deserialize)]
struct ComparisonResult {
    version_a: String,
    version_b: String,
    difference_percent: f64,
    total_chunks_a: usize,
    total_chunks_b: usize,
    common_chunks: usize,
    different_chunks: usize,
    method: String,
    processing_time_ms: u128,
    processing_time_us: u128,  // microseconds
    processing_time_ns: u128,  // nanoseconds
    line_diffs: Vec<LineDiff>,
}

#[derive(Debug, Clone)]
struct MerkleNode {
    hash: String,
    left: Option<Arc<MerkleNode>>,
    right: Option<Arc<MerkleNode>>,
}

// Optimized MerkleNode creation
impl MerkleNode {
    fn new_leaf(hash: String) -> Self {
        MerkleNode {
            hash,
            left: None,
            right: None,
        }
    }
    
    fn new_internal(hash: String, left: Arc<MerkleNode>, right: Option<Arc<MerkleNode>>) -> Self {
        MerkleNode {
            hash,
            left: Some(left),
            right,
        }
    }
}

#[derive(Debug, Clone)]
struct TokenWithLine {
    content: String,
    line_number: usize,
}

fn normalize_html_with_lines(html: &str, chunk_size: usize) -> (Vec<String>, Vec<TokenWithLine>) {
    // Pre-allocate with estimated capacity to reduce reallocations
    let estimated_tokens = html.len() / 20; // Rough estimate
    let mut tokens_with_lines = Vec::with_capacity(estimated_tokens);
    let tag_re = Regex::new(r"<[^>]+>").unwrap();
    
    let mut last_end = 0;
    
    // Pre-compute line positions for better performance
    let line_positions: Vec<usize> = html.char_indices()
        .filter_map(|(i, c)| if c == '\n' { Some(i) } else { None })
        .collect();
    
    // Count newlines up to each position using binary search
    fn count_lines_up_to(line_positions: &[usize], pos: usize) -> usize {
        match line_positions.binary_search(&pos) {
            Ok(idx) => idx + 2, // Found exact position, line number is index + 2
            Err(idx) => idx + 1, // Insert position gives us the line number
        }
    }
    
    for mat in tag_re.find_iter(html) {
        // Add text content before this tag
        if mat.start() > last_end {
            let text = html[last_end..mat.start()].trim();
            if !text.is_empty() {
                let line_num = count_lines_up_to(&line_positions, mat.start());
                // Use String::with_capacity to reduce allocations
                let mut token_content = String::with_capacity(5 + text.len());
                token_content.push_str("TEXT:");
                token_content.push_str(text);
                tokens_with_lines.push(TokenWithLine {
                    content: token_content,
                    line_number: line_num,
                });
            }
        }
        
        // Add the tag itself, normalized
        let tag = mat.as_str().trim();
        if !tag.is_empty() {
            let line_num = count_lines_up_to(&line_positions, mat.start());
            // Optimize tag normalization to avoid collect/join
            let mut normalized_tag = String::with_capacity(tag.len());
            let mut first = true;
            for word in tag.split_whitespace() {
                if !first {
                    normalized_tag.push(' ');
                }
                normalized_tag.push_str(word);
                first = false;
            }
            let mut token_content = String::with_capacity(4 + normalized_tag.len());
            token_content.push_str("TAG:");
            token_content.push_str(&normalized_tag);
            tokens_with_lines.push(TokenWithLine {
                content: token_content,
                line_number: line_num,
            });
        }
        
        last_end = mat.end();
    }
    
    // Add any remaining text after the last tag
    if last_end < html.len() {
        let text = html[last_end..].trim();
        if !text.is_empty() {
            let line_num = count_lines_up_to(&line_positions, html.len());
            let mut token_content = String::with_capacity(5 + text.len());
            token_content.push_str("TEXT:");
            token_content.push_str(text);
            tokens_with_lines.push(TokenWithLine {
                content: token_content,
                line_number: line_num,
            });
        }
    }
    
    // Group tokens into chunks of specified size
    let tokens: Vec<String> = tokens_with_lines.iter().map(|t| t.content.clone()).collect();
    
    let chunks = if chunk_size <= 1 {
        tokens
    } else {
        let mut chunks = Vec::new();
        for chunk in tokens.chunks(chunk_size) {
            let combined_chunk = chunk.join("|");
            chunks.push(combined_chunk);
        }
        chunks
    };
    
    (chunks, tokens_with_lines)
}

fn normalize_html(html: &str, chunk_size: usize) -> Vec<String> {
    let (chunks, _) = normalize_html_with_lines(html, chunk_size);
    chunks
}

// Fast non-cryptographic hash for performance
fn hash_chunk_fast(chunk: &str) -> String {
    format!("{:x}", xxh3_64(chunk.as_bytes()))
}


// Default to fast hash with caching
fn hash_chunk(chunk: &str) -> String {
    // Check cache first
    if let Ok(cache) = HASH_CACHE.lock() {
        if let Some(cached_hash) = cache.get(chunk) {
            return cached_hash.clone();
        }
    }
    
    let hash = hash_chunk_fast(chunk);
    
    // Store in cache
    if let Ok(mut cache) = HASH_CACHE.lock() {
        // Simple cache size limit to prevent unbounded growth
        if cache.len() > 10000 {
            cache.clear(); // Simple eviction strategy
        }
        cache.insert(chunk.to_string(), hash.clone());
    }
    
    hash
}


// Configurable chunk hashing (parallel or sequential)
fn hash_chunks(chunks: &[String]) -> Vec<String> {
    let config = get_perf_config();
    if config.use_parallel_hashing {
        chunks.par_iter().map(|c| hash_chunk(c)).collect()
    } else {
        chunks.iter().map(|c| hash_chunk(c)).collect()
    }
}


// Merkle Lite: Simple hashing approach (current implementation)
fn merkle_lite_hash(chunks: &[String]) -> Vec<String> {
    hash_chunks(chunks)
}

// Optimized Full Merkle Tree implementation with Arc to avoid cloning
fn build_merkle_tree(chunks: &[String]) -> Option<Arc<MerkleNode>> {
    if chunks.is_empty() {
        return None;
    }
    
    // Use parallel iterator for leaf node creation
    let mut nodes: Vec<Arc<MerkleNode>> = chunks
        .par_iter()
        .map(|chunk| Arc::new(MerkleNode::new_leaf(hash_chunk(chunk))))
        .collect();
    
    while nodes.len() > 1 {
        let mut next_level = Vec::new();
        
        for i in (0..nodes.len()).step_by(2) {
            let left = nodes[i].clone(); // Arc clone is cheap
            let right = if i + 1 < nodes.len() {
                Some(nodes[i + 1].clone())
            } else {
                None
            };
            
            let combined_hash = if let Some(ref r) = right {
                hash_chunk(&format!("{}{}", left.hash, r.hash))
            } else {
                left.hash.clone()
            };
            
            next_level.push(Arc::new(MerkleNode::new_internal(combined_hash, left, right)));
        }
        
        nodes = next_level;
    }
    
    nodes.into_iter().next()
}

fn extract_merkle_hashes(node: &Arc<MerkleNode>) -> Vec<String> {
    let mut hashes = Vec::new();
    
    // Collect leaf node hashes with optimized traversal
    fn collect_leaves(node: &Arc<MerkleNode>, hashes: &mut Vec<String>) {
        if node.left.is_none() && node.right.is_none() {
            // Leaf node
            hashes.push(node.hash.clone());
        } else {
            if let Some(ref left) = node.left {
                collect_leaves(left, hashes);
            }
            if let Some(ref right) = node.right {
                collect_leaves(right, hashes);
            }
        }
    }
    
    collect_leaves(node, &mut hashes);
    hashes
}

fn merkle_tree_hash(chunks: &[String]) -> Vec<String> {
    if let Some(tree) = build_merkle_tree(chunks) {
        extract_merkle_hashes(&tree)
    } else {
        Vec::new()
    }
}

fn detailed_diff(hashes_a: &[String], hashes_b: &[String]) -> (f64, usize, usize, usize, usize) {
    let set_a: HashSet<_> = hashes_a.iter().collect();
    let set_b: HashSet<_> = hashes_b.iter().collect();

    let total = set_a.union(&set_b).count();
    let common = set_a.intersection(&set_b).count();
    let diff = set_a.symmetric_difference(&set_b).count();

    let percent = if total == 0 {
        0.0
    } else {
        (diff as f64 / total as f64) * 100.0
    };

    (percent, hashes_a.len(), hashes_b.len(), common, diff)
}

fn generate_line_diffs(tokens_a: &[TokenWithLine], tokens_b: &[TokenWithLine]) -> Vec<LineDiff> {
    let mut line_diffs = Vec::new();
    
    // Create hash sets for comparison
    let set_a: HashSet<_> = tokens_a.iter().map(|t| &t.content).collect();
    let set_b: HashSet<_> = tokens_b.iter().map(|t| &t.content).collect();
    
    // Find added tokens (in b but not in a) with content
    let mut added_items = Vec::new();
    for token in tokens_b {
        if !set_a.contains(&token.content) {
            added_items.push((token.line_number, token.content.clone()));
        }
    }
    
    // Find removed tokens (in a but not in b) with content
    let mut removed_items = Vec::new();
    for token in tokens_a {
        if !set_b.contains(&token.content) {
            removed_items.push((token.line_number, token.content.clone()));
        }
    }
    
    // Group consecutive line numbers into ranges with content
    fn group_consecutive_lines_with_content(mut items: Vec<(usize, String)>) -> Vec<(String, String)> {
        if items.is_empty() {
            return Vec::new();
        }
        
        items.sort_by_key(|&(line, _)| line);
        
        let mut ranges = Vec::new();
        let mut start = items[0].0;
        let mut end = items[0].0;
        let mut content_samples = vec![items[0].1.clone()];
        
        for &(line, ref content) in &items[1..] {
            if line == end + 1 {
                end = line;
                if content_samples.len() < 3 {
                    content_samples.push(content.clone());
                }
            } else {
                let range = if start == end {
                    format!("L{}", start)
                } else {
                    format!("L{}-L{}", start, end)
                };
                
                let preview = if content_samples.len() <= 2 {
                    content_samples.join(", ")
                } else {
                    format!("{}, {} ... ({} more)", 
                        content_samples[0], content_samples[1], content_samples.len() - 2)
                };
                
                ranges.push((range, preview));
                start = line;
                end = line;
                content_samples = vec![content.clone()];
            }
        }
        
        // Add the last range
        let range = if start == end {
            format!("L{}", start)
        } else {
            format!("L{}-L{}", start, end)
        };
        
        let preview = if content_samples.len() <= 2 {
            content_samples.join(", ")
        } else {
            format!("{}, {} ... ({} more)", 
                content_samples[0], content_samples[1], content_samples.len() - 2)
        };
        
        ranges.push((range, preview));
        ranges
    }
    
    // Generate line diffs for added content
    for (range, content) in group_consecutive_lines_with_content(added_items) {
        line_diffs.push(LineDiff {
            line_range: range,
            change_type: "added".to_string(),
            content_preview: format!("+ {}", content.chars().take(100).collect::<String>()),
        });
    }
    
    // Generate line diffs for removed content
    for (range, content) in group_consecutive_lines_with_content(removed_items) {
        line_diffs.push(LineDiff {
            line_range: range,
            change_type: "removed".to_string(),
            content_preview: format!("- {}", content.chars().take(100).collect::<String>()),
        });
    }
    
    line_diffs
}

fn compare_with_method(chunks_a: &[String], chunks_b: &[String], use_merkle_tree: bool) -> (f64, usize, usize, usize, usize, u128, u128, u128) {
    let start = Instant::now();
    
    let (hashes_a, hashes_b) = if use_merkle_tree {
        (merkle_tree_hash(chunks_a), merkle_tree_hash(chunks_b))
    } else {
        (merkle_lite_hash(chunks_a), merkle_lite_hash(chunks_b))
    };
    
    let (percent, total_a, total_b, common, different) = detailed_diff(&hashes_a, &hashes_b);
    let elapsed = start.elapsed();
    let duration_ms = elapsed.as_millis();
    let duration_us = elapsed.as_micros();
    let duration_ns = elapsed.as_nanos();
    
    (percent, total_a, total_b, common, different, duration_ms, duration_us, duration_ns)
}

fn compare_with_line_diffs(content_a: &str, content_b: &str, chunk_size: usize, use_merkle_tree: bool) -> (f64, usize, usize, usize, usize, u128, u128, u128, Vec<LineDiff>) {
    let start = Instant::now();
    
    let (chunks_a, tokens_a) = normalize_html_with_lines(content_a, chunk_size);
    let (chunks_b, tokens_b) = normalize_html_with_lines(content_b, chunk_size);
    
    let (hashes_a, hashes_b) = if use_merkle_tree {
        (merkle_tree_hash(&chunks_a), merkle_tree_hash(&chunks_b))
    } else {
        (merkle_lite_hash(&chunks_a), merkle_lite_hash(&chunks_b))
    };
    
    let (percent, total_a, total_b, common, different) = detailed_diff(&hashes_a, &hashes_b);
    let line_diffs = generate_line_diffs(&tokens_a, &tokens_b);
    let elapsed = start.elapsed();
    let duration_ms = elapsed.as_millis();
    let duration_us = elapsed.as_micros();
    let duration_ns = elapsed.as_nanos();
    
    (percent, total_a, total_b, common, different, duration_ms, duration_us, duration_ns, line_diffs)
}

fn process_file<P: AsRef<Path>>(path: P, chunk_size: usize) -> Vec<String> {
    let content = fs::read_to_string(path).expect("Failed to read file");
    let chunks = normalize_html(&content, chunk_size);
    hash_chunks(&chunks)
}

fn generate_random_dom_with_changes(base_content: &str, version: usize) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    
    // Create a pseudo-random seed from version number
    let mut hasher = DefaultHasher::new();
    version.hash(&mut hasher);
    let seed = hasher.finish();
    
    let mut modified = base_content.to_string();
    
    // Random small changes based on seeded pseudo-randomness
    let changes = vec![
        // Add random attributes
        (seed % 7 == 0, format!(" data-rand='{}'", seed % 1000)),
        // Add random classes
        (seed % 11 == 0, format!(" class='gen-{}'", seed % 100)),
        // Add random IDs
        (seed % 13 == 0, format!(" id='elem-{}'", seed % 500)),
        // Add random text nodes
        (seed % 17 == 0, format!("Random text {}", seed % 50)),
        // Add random comments
        (seed % 19 == 0, format!("<!-- Random comment {} -->", seed % 200)),
    ];
    
    // Apply random changes
    for (should_apply, change) in &changes {
        if *should_apply {
            // Insert change at random position based on seed
            let insertion_point = (seed as usize) % modified.len().max(1);
            modified.insert_str(insertion_point, change);
        }
    }
    
    // Add some structural changes
    if seed % 23 == 0 {
        modified.push_str(&format!("<div><span>Generated {}</span></div>", version));
    }
    
    if seed % 29 == 0 {
        modified = modified.replace("</head>", &format!("<meta name='version' content='{}'></head>", version));
    }
    
    if seed % 31 == 0 {
        modified.push_str(&format!("<script>var version = {};</script>", version));
    }
    
    // Small text modifications
    if seed % 37 == 0 {
        modified = modified.replace("div", &format!("div{}", seed % 10));
    }
    
    modified
}

fn generate_random_comparisons(base_content: &str, num_comparisons: usize, chunk_size: usize, use_merkle_tree: bool, include_line_diffs: bool) -> Vec<ComparisonResult> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    
    let mut results = Vec::new();
    
    // First, generate all versions (1-100)
    let mut versions = Vec::new();
    versions.push(("base".to_string(), base_content.to_string()));
    
    for i in 1..=100 {
        let version_content = if i % 7 == 0 || i % 11 == 0 {
            // Keep some versions unchanged
            base_content.to_string()
        } else {
            generate_random_dom_with_changes(base_content, i)
        };
        versions.push((format!("v{}", i), version_content));
    }
    
    // Generate random comparisons
    for comparison_idx in 0..num_comparisons {
        // Create pseudo-random indices based on comparison index
        let mut hasher = DefaultHasher::new();
        comparison_idx.hash(&mut hasher);
        let seed = hasher.finish();
        
        let idx_a = (seed % versions.len() as u64) as usize;
        let idx_b = ((seed / versions.len() as u64) % versions.len() as u64) as usize;
        
        // Skip if comparing same version
        if idx_a == idx_b {
            continue;
        }
        
        let (name_a, content_a) = &versions[idx_a];
        let (name_b, content_b) = &versions[idx_b];
        
        let (percent, total_a, total_b, common, different, duration_ms, duration_us, duration_ns, line_diffs) = if include_line_diffs {
            compare_with_line_diffs(content_a, content_b, chunk_size, use_merkle_tree)
        } else {
            let chunks_a = normalize_html(content_a, chunk_size);
            let chunks_b = normalize_html(content_b, chunk_size);
            let (p, ta, tb, c, d, dur_ms, dur_us, dur_ns) = compare_with_method(&chunks_a, &chunks_b, use_merkle_tree);
            (p, ta, tb, c, d, dur_ms, dur_us, dur_ns, Vec::new())
        };
        
        results.push(ComparisonResult {
            version_a: name_a.clone(),
            version_b: name_b.clone(),
            difference_percent: percent,
            total_chunks_a: total_a,
            total_chunks_b: total_b,
            common_chunks: common,
            different_chunks: different,
            method: if use_merkle_tree { "merkle_tree".to_string() } else { "merkle_lite".to_string() },
            processing_time_ms: duration_ms,
            processing_time_us: duration_us,
            processing_time_ns: duration_ns,
            line_diffs,
        });
    }
    
    results
}

fn get_memory_usage() -> usize {
    // This is a simplified memory usage estimation
    // In practice, you'd use a proper memory profiling tool
    use std::process::Command;
    
    let output = Command::new("ps")
        .args(&["-o", "rss=", "-p", &std::process::id().to_string()])
        .output();
    
    if let Ok(output) = output {
        let rss_str = String::from_utf8_lossy(&output.stdout);
        rss_str.trim().parse::<usize>().unwrap_or(0) * 1024 // Convert KB to bytes
    } else {
        0
    }
}

#[derive(Serialize)]
struct BenchmarkResult {
    method: String,
    chunk_size: usize,
    num_tests: usize,
    avg_time_ms: f64,
    avg_time_us: f64,  // microseconds
    avg_time_ns: f64,  // nanoseconds
    min_time_ms: u128,
    min_time_us: u128,
    min_time_ns: u128,
    max_time_ms: u128,
    max_time_us: u128,
    max_time_ns: u128,
    total_time_ms: u128,
    total_time_us: u128,
    total_time_ns: u128,
    memory_usage_bytes: usize,
    throughput_comparisons_per_sec: f64,
}

fn run_benchmark(num_tests: usize) {
    println!("Running benchmark with {} tests...", num_tests);
    
    let base_html = r##"<!DOCTYPE html>
<html>
<head>
    <title>Benchmark HTML</title>
    <meta charset="utf-8">
    <style>
        body { font-family: Arial, sans-serif; }
        .container { max-width: 1200px; margin: 0 auto; }
        .header { background: #333; color: white; padding: 20px; }
        .content { padding: 20px; }
        .footer { background: #666; color: white; padding: 10px; }
    </style>
</head>
<body>
    <div class="container">
        <header class="header">
            <h1>Performance Test Page</h1>
            <nav>
                <ul>
                    <li><a href="#home">Home</a></li>
                    <li><a href="#about">About</a></li>
                    <li><a href="#contact">Contact</a></li>
                </ul>
            </nav>
        </header>
        <main class="content">
            <section>
                <h2>Content Section</h2>
                <p>This is a test paragraph with <strong>bold</strong> and <em>italic</em> text.</p>
                <div class="card">
                    <h3>Card Title</h3>
                    <p>Card content goes here with more text to analyze.</p>
                    <button onclick="alert('clicked')">Click Me</button>
                </div>
            </section>
            <section>
                <h2>List Section</h2>
                <ul>
                    <li>Item 1</li>
                    <li>Item 2</li>
                    <li>Item 3</li>
                </ul>
                <table>
                    <tr><th>Column 1</th><th>Column 2</th></tr>
                    <tr><td>Data 1</td><td>Data 2</td></tr>
                </table>
            </section>
        </main>
        <footer class="footer">
            <p>&copy; 2025 Benchmark Test</p>
        </footer>
    </div>
    <script>
        console.log('Page loaded');
        function testFunction() {
            return 'test';
        }
    </script>
</body>
</html>"##;
    
    let chunk_sizes = vec![1, 2, 3, 5];
    let methods = vec![("merkle_lite", false), ("merkle_tree", true)];
    
    let mut benchmark_results = Vec::new();
    
    for chunk_size in chunk_sizes {
        for (method_name, use_merkle_tree) in &methods {
            println!("Testing {} with chunk size {}...", method_name, chunk_size);
            
            let mut times = Vec::new();
            let memory_before = get_memory_usage();
            
            let total_start = Instant::now();
            
            for test_idx in 0..num_tests {
                // Generate a slightly different version for each test
                let modified_html = format!("{}<div id='test-{}'></div>", base_html, test_idx);
                
                let (chunks_a, _) = normalize_html_with_lines(base_html, chunk_size);
                let (chunks_b, _) = normalize_html_with_lines(&modified_html, chunk_size);
                
                // Use black_box to prevent compiler optimizations
                let (_percent, _total_a, _total_b, _common, _different, duration_ms, duration_us, duration_ns) = 
                    compare_with_method(
                        &black_box(chunks_a), 
                        &black_box(chunks_b), 
                        black_box(*use_merkle_tree)
                    );
                
                times.push(black_box(duration_ms));
            }
            
            let total_elapsed = total_start.elapsed();
            let total_duration_ms = total_elapsed.as_millis();
            let total_duration_us = total_elapsed.as_micros();
            let total_duration_ns = total_elapsed.as_nanos();
            let memory_after = get_memory_usage();
            
            let avg_time_ms = times.iter().sum::<u128>() as f64 / times.len() as f64;
            let min_time_ms = *times.iter().min().unwrap_or(&0);
            let max_time_ms = *times.iter().max().unwrap_or(&0);
            
            // Calculate averages for all time units
            let avg_time_us = avg_time_ms * 1000.0;
            let avg_time_ns = avg_time_us * 1000.0;
            let min_time_us = min_time_ms * 1000;
            let min_time_ns = min_time_us * 1000;
            let max_time_us = max_time_ms * 1000;
            let max_time_ns = max_time_us * 1000;
            
            let throughput = if total_duration_ms > 0 {
                (num_tests as f64 * 1000.0) / total_duration_ms as f64
            } else {
                0.0
            };
            
            benchmark_results.push(BenchmarkResult {
                method: method_name.to_string(),
                chunk_size,
                num_tests,
                avg_time_ms: avg_time_ms,
                avg_time_us: avg_time_us,
                avg_time_ns: avg_time_ns,
                min_time_ms: min_time_ms,
                min_time_us: min_time_us,
                min_time_ns: min_time_ns,
                max_time_ms: max_time_ms,
                max_time_us: max_time_us,
                max_time_ns: max_time_ns,
                total_time_ms: total_duration_ms,
                total_time_us: total_duration_us,
                total_time_ns: total_duration_ns,
                memory_usage_bytes: memory_after.saturating_sub(memory_before),
                throughput_comparisons_per_sec: throughput,
            });
        }
    }
    
    // Create result directory if it doesn't exist
    fs::create_dir_all("result").unwrap_or_else(|_| {
        eprintln!("Error: Could not create result directory");
        std::process::exit(1);
    });
    
    // Generate timestamp
    let now: DateTime<Utc> = Utc::now();
    let timestamp = now.format("%Y%m%d_%H%M%S").to_string();
    let filename = format!("result/benchmark-{}.json", timestamp);
    
    // Save benchmark results
    let json_output = serde_json::to_string_pretty(&benchmark_results).expect("Failed to serialize benchmark results");
    fs::write(&filename, &json_output).unwrap_or_else(|_| {
        eprintln!("Error: Could not write benchmark file {}", filename);
        std::process::exit(1);
    });
    
    // Print summary
    println!("\n=== BENCHMARK RESULTS ===");
    println!("Benchmark results saved to: {}", filename);
    println!();
    
    for result in &benchmark_results {
        println!("Method: {} (chunk size: {})", result.method, result.chunk_size);
        
        // Choose best unit for display based on timing
        if result.avg_time_ms >= 1.0 {
            println!("  Average time: {:.3} ms ({:.1} μs, {:.0} ns)", result.avg_time_ms, result.avg_time_us, result.avg_time_ns);
            println!("  Min/Max time: {} ms / {} ms", result.min_time_ms, result.max_time_ms);
        } else if result.avg_time_us >= 1.0 {
            println!("  Average time: {:.3} μs ({:.3} ms, {:.0} ns)", result.avg_time_us, result.avg_time_ms, result.avg_time_ns);
            println!("  Min/Max time: {} μs / {} μs", result.min_time_us, result.max_time_us);
        } else {
            println!("  Average time: {:.0} ns ({:.3} μs, {:.3} ms)", result.avg_time_ns, result.avg_time_us, result.avg_time_ms);
            println!("  Min/Max time: {} ns / {} ns", result.min_time_ns, result.max_time_ns);
        }
        
        println!("  Total time: {} ms ({} μs, {} ns)", result.total_time_ms, result.total_time_us, result.total_time_ns);
        println!("  Memory usage: {} bytes ({:.2} KB)", result.memory_usage_bytes, result.memory_usage_bytes as f64 / 1024.0);
        println!("  Throughput: {:.2} comparisons/sec", result.throughput_comparisons_per_sec);
        println!();
    }
    
    // Performance comparison
    println!("=== PERFORMANCE COMPARISON ===");
    for chunk_size in vec![1, 2, 3, 5] {
        let lite_result = benchmark_results.iter().find(|r| r.method == "merkle_lite" && r.chunk_size == chunk_size);
        let tree_result = benchmark_results.iter().find(|r| r.method == "merkle_tree" && r.chunk_size == chunk_size);
        
        if let (Some(lite), Some(tree)) = (lite_result, tree_result) {
            let speed_ratio = if tree.avg_time_ms > 0.0 { 
                lite.avg_time_ms / tree.avg_time_ms 
            } else { 
                1.0 
            };
            let winner = if lite.avg_time_ms < tree.avg_time_ms { "Merkle Lite" } else { "Merkle Tree" };
            
            println!("Chunk Size {}: {} is {:.2}x faster", chunk_size, winner, speed_ratio.max(1.0 / speed_ratio));
            
            // Show timing in most appropriate unit
            if lite.avg_time_ms >= 1.0 {
                println!("  Merkle Lite: {:.3} ms avg, {:.2} comparisons/sec", lite.avg_time_ms, lite.throughput_comparisons_per_sec);
                println!("  Merkle Tree: {:.3} ms avg, {:.2} comparisons/sec", tree.avg_time_ms, tree.throughput_comparisons_per_sec);
            } else {
                println!("  Merkle Lite: {:.1} μs avg, {:.2} comparisons/sec", lite.avg_time_us, lite.throughput_comparisons_per_sec);
                println!("  Merkle Tree: {:.1} μs avg, {:.2} comparisons/sec", tree.avg_time_us, tree.throughput_comparisons_per_sec);
            }
            println!();
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    
    if args.len() >= 2 {
        match args[1].as_str() {
            "--generate-dom" => {
                if args.len() != 4 {
                    eprintln!("Usage: {} --generate-dom <base_file> <num_versions>", args[0]);
                    std::process::exit(1);
                }
                
                let base_file = &args[2];
                let num_versions: usize = args[3].parse().unwrap_or_else(|_| {
                    eprintln!("Error: num_versions must be a number");
                    std::process::exit(1);
                });
                
                let base_content = fs::read_to_string(base_file).unwrap_or_else(|_| {
                    eprintln!("Error: Could not read base file {}", base_file);
                    std::process::exit(1);
                });
                
                // Generate DOM versions and save to files
                for i in 1..=num_versions {
                    let version_content = if i % 7 == 0 || i % 11 == 0 {
                        base_content.clone()
                    } else {
                        generate_random_dom_with_changes(&base_content, i)
                    };
                    
                    let filename = format!("v{}.html", i);
                    fs::write(&filename, version_content).unwrap_or_else(|_| {
                        eprintln!("Error: Could not write file {}", filename);
                        std::process::exit(1);
                    });
                }
                
                println!("Generated {} DOM versions", num_versions);
                return;
            },
            
            "--compare-random" => {
                if args.len() < 3 || args.len() > 4 {
                    eprintln!("Usage: {} --compare-random <num_comparisons> [chunk_size]", args[0]);
                    eprintln!("  chunk_size: Number of tokens per chunk (default: 1)");
                    eprintln!("  Note: This command includes line diffs in JSON output");
                    std::process::exit(1);
                }
                
                let num_comparisons: usize = args[2].parse().unwrap_or_else(|_| {
                    eprintln!("Error: num_comparisons must be a number");
                    std::process::exit(1);
                });
                
                let chunk_size: usize = if args.len() == 4 {
                    args[3].parse().unwrap_or_else(|_| {
                        eprintln!("Error: chunk_size must be a number");
                        std::process::exit(1);
                    })
                } else {
                    1 // Default chunk size
                };
                
                println!("Generating {} random comparisons with line diffs (this may take longer)...", num_comparisons);
                
                let base_html = r#"<!DOCTYPE html>
<html>
<head>
    <title>Base HTML</title>
</head>
<body>
    <div>
        <h1>Welcome</h1>
        <p>This is the base content.</p>
        <ul>
            <li>Item 1</li>
            <li>Item 2</li>
        </ul>
    </div>
</body>
</html>"#;
                
                let results = generate_random_comparisons(base_html, num_comparisons, chunk_size, false, true);
                let json_output = serde_json::to_string_pretty(&results).expect("Failed to serialize to JSON");
                
                // Create result directory if it doesn't exist
                fs::create_dir_all("result").unwrap_or_else(|_| {
                    eprintln!("Error: Could not create result directory");
                    std::process::exit(1);
                });
                
                // Generate timestamp
                let now: DateTime<Utc> = Utc::now();
                let timestamp = now.format("%Y%m%d_%H%M%S").to_string();
                let filename = format!("result/run-{}-chunks{}-with-lines.json", timestamp, chunk_size);
                
                // Save to file
                fs::write(&filename, &json_output).unwrap_or_else(|_| {
                    eprintln!("Error: Could not write result file {}", filename);
                    std::process::exit(1);
                });
                
                println!("Results saved to: {}", filename);
                println!("Generated {} random comparisons with chunk size {} (including line diffs)", num_comparisons, chunk_size);
                return;
            },
            
            "--compare-random-fast" => {
                if args.len() < 3 || args.len() > 4 {
                    eprintln!("Usage: {} --compare-random-fast <num_comparisons> [chunk_size]", args[0]);
                    eprintln!("  chunk_size: Number of tokens per chunk (default: 1)");
                    eprintln!("  Note: Fast mode without line diffs");
                    std::process::exit(1);
                }
                
                let num_comparisons: usize = args[2].parse().unwrap_or_else(|_| {
                    eprintln!("Error: num_comparisons must be a number");
                    std::process::exit(1);
                });
                
                let chunk_size: usize = if args.len() == 4 {
                    args[3].parse().unwrap_or_else(|_| {
                        eprintln!("Error: chunk_size must be a number");
                        std::process::exit(1);
                    })
                } else {
                    1 // Default chunk size
                };
                
                let base_html = r#"<!DOCTYPE html>
<html>
<head>
    <title>Base HTML</title>
</head>
<body>
    <div>
        <h1>Welcome</h1>
        <p>This is the base content.</p>
        <ul>
            <li>Item 1</li>
            <li>Item 2</li>
        </ul>
    </div>
</body>
</html>"#;
                
                let results = generate_random_comparisons(base_html, num_comparisons, chunk_size, false, false);
                let json_output = serde_json::to_string_pretty(&results).expect("Failed to serialize to JSON");
                
                // Create result directory if it doesn't exist
                fs::create_dir_all("result").unwrap_or_else(|_| {
                    eprintln!("Error: Could not create result directory");
                    std::process::exit(1);
                });
                
                // Generate timestamp
                let now: DateTime<Utc> = Utc::now();
                let timestamp = now.format("%Y%m%d_%H%M%S").to_string();
                let filename = format!("result/run-{}-chunks{}-fast.json", timestamp, chunk_size);
                
                // Save to file
                fs::write(&filename, &json_output).unwrap_or_else(|_| {
                    eprintln!("Error: Could not write result file {}", filename);
                    std::process::exit(1);
                });
                
                println!("Results saved to: {}", filename);
                println!("Generated {} random comparisons with chunk size {} (fast mode)", num_comparisons, chunk_size);
                return;
            },
            
            "--benchmark" => {
                if args.len() != 3 {
                    eprintln!("Usage: {} --benchmark <num_tests>", args[0]);
                    std::process::exit(1);
                }
                
                let num_tests: usize = args[2].parse().unwrap_or_else(|_| {
                    eprintln!("Error: num_tests must be a number");
                    std::process::exit(1);
                });
                
                run_benchmark(num_tests);
                return;
            },
            
            "--line-diff" => {
                if args.len() < 4 || args.len() > 5 {
                    eprintln!("Usage: {} --line-diff <file1.html> <file2.html> [chunk_size]", args[0]);
                    eprintln!("  Generates detailed line-by-line diff with L100-L120 format");
                    std::process::exit(1);
                }
                
                let file1 = &args[2];
                let file2 = &args[3];
                let chunk_size: usize = if args.len() == 5 {
                    args[4].parse().unwrap_or_else(|_| {
                        eprintln!("Error: chunk_size must be a number");
                        std::process::exit(1);
                    })
                } else {
                    1 // Default chunk size
                };
                
                let content1 = fs::read_to_string(file1).unwrap_or_else(|_| {
                    eprintln!("Error: Could not read file {}", file1);
                    std::process::exit(1);
                });
                
                let content2 = fs::read_to_string(file2).unwrap_or_else(|_| {
                    eprintln!("Error: Could not read file {}", file2);
                    std::process::exit(1);
                });
                
                let (percent, total_a, total_b, common, different, duration_ms, duration_us, duration_ns, line_diffs) = 
                    compare_with_line_diffs(&content1, &content2, chunk_size, false);
                
                println!("=== LINE DIFF ANALYSIS ===");
                println!("Files: {} vs {}", file1, file2);
                println!("Overall difference: {:.2}%", percent);
                
                // Display timing in most appropriate unit
                if duration_ms >= 1 {
                    println!("Processing time: {} ms ({} μs, {} ns)", duration_ms, duration_us, duration_ns);
                } else if duration_us >= 1 {
                    println!("Processing time: {} μs ({} ms, {} ns)", duration_us, duration_ms, duration_ns);
                } else {
                    println!("Processing time: {} ns ({} μs, {} ms)", duration_ns, duration_us, duration_ms);
                }
                println!("Total chunks: {} vs {}", total_a, total_b);
                println!("Common chunks: {}, Different chunks: {}", common, different);
                println!();
                
                if line_diffs.is_empty() {
                    println!("No line-level differences found.");
                } else {
                    println!("=== LINE-BY-LINE CHANGES ===");
                    for diff in &line_diffs {
                        println!("{}: {}", diff.line_range, diff.content_preview);
                    }
                    println!();
                    println!("Total line changes: {}", line_diffs.len());
                }
                
                // Save detailed results to JSON
                fs::create_dir_all("result").unwrap_or_else(|_| {
                    eprintln!("Error: Could not create result directory");
                    std::process::exit(1);
                });
                
                let now: DateTime<Utc> = Utc::now();
                let timestamp = now.format("%Y%m%d_%H%M%S").to_string();
                let filename = format!("result/line-diff-{}.json", timestamp);
                
                let result = ComparisonResult {
                    version_a: file1.to_string(),
                    version_b: file2.to_string(),
                    difference_percent: percent,
                    total_chunks_a: total_a,
                    total_chunks_b: total_b,
                    common_chunks: common,
                    different_chunks: different,
                    method: "merkle_lite_with_lines".to_string(),
                    processing_time_ms: duration_ms,
                    processing_time_us: duration_us,
                    processing_time_ns: duration_ns,
                    line_diffs,
                };
                
                let json_output = serde_json::to_string_pretty(&result).expect("Failed to serialize to JSON");
                fs::write(&filename, &json_output).unwrap_or_else(|_| {
                    eprintln!("Error: Could not write result file {}", filename);
                    std::process::exit(1);
                });
                
                println!("Detailed results saved to: {}", filename);
                return;
            },
            
            _ => {}
        }
    }
    
    if args.len() < 3 || args.len() > 4 {
        eprintln!("Usage: {} <file1.html> <file2.html> [chunk_size]", args[0]);
        eprintln!("   or: {} --generate-dom <base_file> <num_versions>", args[0]);
        eprintln!("   or: {} --compare-random <num_comparisons> [chunk_size]  (with line diffs)", args[0]);
        eprintln!("   or: {} --compare-random-fast <num_comparisons> [chunk_size]  (without line diffs)", args[0]);
        eprintln!("   or: {} --benchmark <num_tests>", args[0]);
        eprintln!("   or: {} --line-diff <file1.html> <file2.html> [chunk_size]", args[0]);
        eprintln!("  chunk_size: Number of tokens per chunk (default: 1)");
        std::process::exit(1);
    }

    let file1 = &args[1];
    let file2 = &args[2];
    
    let chunk_size: usize = if args.len() == 4 {
        args[3].parse().unwrap_or_else(|_| {
            eprintln!("Error: chunk_size must be a number");
            std::process::exit(1);
        })
    } else {
        1 // Default chunk size
    };

    let hashes1 = process_file(file1, chunk_size);
    let hashes2 = process_file(file2, chunk_size);

    let (percent, _total_a, _total_b, _common, _different) = detailed_diff(&hashes1, &hashes2);

    println!(
        "DOM diff between {} and {} is {:.2}% (chunk size: {})",
        file1, file2, percent, chunk_size
    );
}
