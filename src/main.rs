use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::Path;
use regex::Regex;

fn normalize_html(html: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let tag_re = Regex::new(r"<[^>]+>").unwrap();
    
    let mut last_end = 0;
    
    for mat in tag_re.find_iter(html) {
        // Add text content before this tag
        if mat.start() > last_end {
            let text = html[last_end..mat.start()].trim();
            if !text.is_empty() {
                tokens.push(format!("TEXT:{}", text));
            }
        }
        
        // Add the tag itself, normalized
        let tag = mat.as_str().trim();
        if !tag.is_empty() {
            // Normalize whitespace within tags
            let normalized_tag = tag.split_whitespace().collect::<Vec<_>>().join(" ");
            tokens.push(format!("TAG:{}", normalized_tag));
        }
        
        last_end = mat.end();
    }
    
    // Add any remaining text after the last tag
    if last_end < html.len() {
        let text = html[last_end..].trim();
        if !text.is_empty() {
            tokens.push(format!("TEXT:{}", text));
        }
    }
    
    tokens
}

fn hash_chunk(chunk: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(chunk.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

fn hash_chunks(chunks: &[String]) -> Vec<String> {
    chunks.iter().map(|c| hash_chunk(c)).collect()
}

fn percent_diff(hashes_a: &[String], hashes_b: &[String]) -> f64 {
    let set_a: HashSet<_> = hashes_a.iter().collect();
    let set_b: HashSet<_> = hashes_b.iter().collect();

    let total = set_a.union(&set_b).count();
    let diff = set_a.symmetric_difference(&set_b).count();

    if total == 0 {
        0.0
    } else {
        (diff as f64 / total as f64) * 100.0
    }
}

fn process_file<P: AsRef<Path>>(path: P) -> Vec<String> {
    let content = fs::read_to_string(path).expect("Failed to read file");
    let chunks = normalize_html(&content);
    hash_chunks(&chunks)
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} v1.html v2.html", args[0]);
        std::process::exit(1);
    }

    let file1 = &args[1];
    let file2 = &args[2];

    let hashes1 = process_file(file1);
    let hashes2 = process_file(file2);

    let percent = percent_diff(&hashes1, &hashes2);

    println!(
        "DOM diff between {} and {} is {:.2}%",
        file1, file2, percent
    );
}
