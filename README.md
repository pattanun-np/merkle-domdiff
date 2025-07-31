# Merkle DOM Diff

A Rust-based tool for comparing DOM structures using merkle tree hashing and generating random DOM variations for testing purposes.

## Features

- **DOM Comparison**: Compare two HTML files and calculate difference percentages
- **Random DOM Generation**: Generate multiple HTML variations from a base file
- **Random Comparison Analysis**: Generate random comparisons between versions with JSON output
- **Merkle Tree Hashing**: Uses SHA-256 hashing for efficient DOM chunk comparison
- **Timestamped Results**: Automatically saves comparison results with timestamps

## Installation

```bash
cargo build --release
```

## Usage

### 1. Compare Two HTML Files

Compare the DOM structure between two specific HTML files:

```bash
cargo run -- file1.html file2.html [chunk_size]
```

**Examples:**
```bash
cargo run -- v1.html v2.html
# Output: DOM diff between v1.html and v2.html is 15.38% (chunk size: 1)

cargo run -- v1.html v2.html 3
# Output: DOM diff between v1.html and v2.html is 25.67% (chunk size: 3)
```

### 2. Generate DOM Variations

Generate multiple HTML variations from a base HTML file:

```bash
cargo run -- --generate-dom <base_file> <num_versions>
```

**Example:**
```bash
cargo run -- --generate-dom base.html 100
# Output: Generated 100 DOM versions
# Creates: v1.html, v2.html, ..., v100.html
```

### 3. Random Comparison Analysis

Generate random comparisons between different versions and save results as JSON:

```bash
cargo run -- --compare-random <num_comparisons> [chunk_size]
```

**Examples:**
```bash
cargo run -- --compare-random 100
# Output: Results saved to: result/run-20250131_143022-chunks1.json
#         Generated 100 random comparisons with chunk size 1

cargo run -- --compare-random 100 5
# Output: Results saved to: result/run-20250131_143055-chunks5.json
#         Generated 100 random comparisons with chunk size 5
```

## Output Format

### JSON Comparison Results

The `--compare-random` command outputs detailed JSON with the following structure:

```json
{
  "version_a": "v25",
  "version_b": "v78",
  "difference_percent": 15.384615384615385,
  "total_chunks_a": 26,
  "total_chunks_b": 28,
  "common_chunks": 22,
  "different_chunks": 4
}
```

**Fields:**
- `version_a`, `version_b`: Version identifiers being compared
- `difference_percent`: Percentage of differences between the versions
- `total_chunks_a`, `total_chunks_b`: Total DOM chunks in each version
- `common_chunks`: Number of identical chunks between versions
- `different_chunks`: Number of differing chunks

### Result Files

Results are automatically saved to timestamped files in the `result/` directory:
- Format: `result/run-YYYYMMDD_HHMMSS-chunks{N}.json`
- Examples: 
  - `result/run-20250131_143022-chunks1.json` (chunk size 1)
  - `result/run-20250131_143055-chunks5.json` (chunk size 5)

## How It Works

### DOM Normalization

1. **Tokenization**: HTML is parsed into tokens (tags and text content)
2. **Normalization**: Whitespace is normalized and tokens are standardized
3. **Chunking**: Tokens are grouped into chunks based on the specified chunk size
4. **Hashing**: Each chunk is hashed using SHA-256 for efficient comparison

### Dynamic Chunk Sizing

The **chunk size** parameter controls how many DOM tokens are grouped together:

- **Chunk Size 1** (default): Each token is analyzed individually
  - Most granular analysis
  - Detects small changes effectively
  - Higher sensitivity to minor modifications

- **Larger Chunk Sizes** (2, 3, 5, etc.): Multiple tokens combined per chunk
  - Less granular but more context-aware
  - Better for detecting structural changes
  - Reduces sensitivity to minor text modifications
  - Fewer total chunks to compare

**Example Impact:**
```bash
# Same files compared with different chunk sizes
cargo run -- v1.html v2.html 1    # 21.40% difference (26 chunks)
cargo run -- v1.html v2.html 2    # 56.56% difference (13 chunks) 
cargo run -- v1.html v2.html 5    # 80.00% difference (5 chunks)
```

### Random Generation

The tool generates pseudo-random DOM variations by:
- Adding random attributes (`data-rand`, `class`, `id`)
- Inserting random text content and comments
- Applying structural changes (divs, scripts, meta tags)
- Some versions remain unchanged (multiples of 7 and 11) to simulate real-world scenarios

### Comparison Algorithm

Uses set-based comparison of hashed DOM chunks:
- **Union**: Total unique chunks across both versions
- **Intersection**: Common chunks between versions
- **Symmetric Difference**: Chunks that differ between versions
- **Percentage**: `(different_chunks / total_unique_chunks) * 100`

## Dependencies

- `sha2`: SHA-256 hashing
- `regex`: HTML parsing and normalization
- `serde`: JSON serialization
- `chrono`: Timestamp generation
- `hex`: Hash encoding

## Examples

### Generate and Compare Workflow

```bash
# 1. Generate 50 DOM variations
cargo run -- --generate-dom base.html 50

# 2. Compare specific versions
cargo run -- v10.html v25.html

# 3. Generate 100 random comparisons
cargo run -- --compare-random 100
```

### Typical Output

```bash
$ cargo run -- --compare-random 5
Results saved to: result/run-20250131_143555.json
Generated 5 random comparisons
```

The generated JSON file contains detailed comparison metrics for analysis and testing purposes.

## Use Cases

- **Web Development**: Testing DOM changes and their impact
- **A/B Testing**: Comparing different versions of web pages  
- **Regression Testing**: Detecting unintended DOM modifications
- **Performance Analysis**: Understanding DOM complexity changes
- **Research**: Analyzing HTML structure variations at scale