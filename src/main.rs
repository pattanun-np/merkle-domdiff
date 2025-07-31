use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::Path;
use regex::Regex;
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

#[derive(Serialize, Deserialize)]
struct ComparisonResult {
    version_a: String,
    version_b: String,
    difference_percent: f64,
    total_chunks_a: usize,
    total_chunks_b: usize,
    common_chunks: usize,
    different_chunks: usize,
}

fn normalize_html(html: &str, chunk_size: usize) -> Vec<String> {
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
    
    // Group tokens into chunks of specified size
    if chunk_size <= 1 {
        return tokens;
    }
    
    let mut chunks = Vec::new();
    for chunk in tokens.chunks(chunk_size) {
        let combined_chunk = chunk.join("|");
        chunks.push(combined_chunk);
    }
    
    chunks
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

fn generate_random_comparisons(base_content: &str, num_comparisons: usize, chunk_size: usize) -> Vec<ComparisonResult> {
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
        
        let chunks_a = normalize_html(content_a, chunk_size);
        let chunks_b = normalize_html(content_b, chunk_size);
        
        let hashes_a = hash_chunks(&chunks_a);
        let hashes_b = hash_chunks(&chunks_b);
        
        let (percent, total_a, total_b, common, different) = detailed_diff(&hashes_a, &hashes_b);
        
        results.push(ComparisonResult {
            version_a: name_a.clone(),
            version_b: name_b.clone(),
            difference_percent: percent,
            total_chunks_a: total_a,
            total_chunks_b: total_b,
            common_chunks: common,
            different_chunks: different,
        });
    }
    
    results
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
                
                let results = generate_random_comparisons(base_html, num_comparisons, chunk_size);
                let json_output = serde_json::to_string_pretty(&results).expect("Failed to serialize to JSON");
                
                // Create result directory if it doesn't exist
                fs::create_dir_all("result").unwrap_or_else(|_| {
                    eprintln!("Error: Could not create result directory");
                    std::process::exit(1);
                });
                
                // Generate timestamp
                let now: DateTime<Utc> = Utc::now();
                let timestamp = now.format("%Y%m%d_%H%M%S").to_string();
                let filename = format!("result/run-{}-chunks{}.json", timestamp, chunk_size);
                
                // Save to file
                fs::write(&filename, &json_output).unwrap_or_else(|_| {
                    eprintln!("Error: Could not write result file {}", filename);
                    std::process::exit(1);
                });
                
                println!("Results saved to: {}", filename);
                println!("Generated {} random comparisons with chunk size {}", num_comparisons, chunk_size);
                return;
            },
            
            _ => {}
        }
    }
    
    if args.len() < 3 || args.len() > 4 {
        eprintln!("Usage: {} <file1.html> <file2.html> [chunk_size]", args[0]);
        eprintln!("   or: {} --generate-dom <base_file> <num_versions>", args[0]);
        eprintln!("   or: {} --compare-random <num_comparisons> [chunk_size]", args[0]);
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
