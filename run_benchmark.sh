#!/bin/bash

# Drusdenx Database Benchmark Runner Script
# ==========================================

set -e  # Exit on error

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
BENCHMARK_DIR="target/criterion"
RESULTS_DIR="benchmark_results"
TIMESTAMP=$(date +"%Y%m%d_%H%M%S")
REPORT_FILE="${RESULTS_DIR}/benchmark_report_${TIMESTAMP}.txt"

# Print banner
print_banner() {
    echo -e "${BLUE}"
    echo "╔═══════════════════════════════════════════════════════╗"
    echo "║           DRUSDENX DATABASE BENCHMARK SUITE          ║"
    echo "╚═══════════════════════════════════════════════════════╝"
    echo -e "${NC}"
}

# Check prerequisites
check_prerequisites() {
    echo -e "${YELLOW}Checking prerequisites...${NC}"
    
    # Check if Rust is installed
    if ! command -v cargo &> /dev/null; then
        echo -e "${RED}✗ Cargo is not installed. Please install Rust.${NC}"
        exit 1
    fi
    
    # Check if criterion is in Cargo.toml
    if ! grep -q "criterion" Cargo.toml; then
        echo -e "${YELLOW}Adding criterion to Cargo.toml...${NC}"
        cat >> Cargo.toml << EOF

[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }
rand = "0.8"

[[bench]]
name = "database_benchmark"
harness = false
EOF
    fi
    
    echo -e "${GREEN}✓ Prerequisites check passed${NC}"
}

# Clean previous benchmark results
clean_previous() {
    echo -e "${YELLOW}Cleaning previous benchmark results...${NC}"
    rm -rf ${BENCHMARK_DIR}
    mkdir -p ${RESULTS_DIR}
    echo -e "${GREEN}✓ Cleaned${NC}"
}

# Build the project in release mode
build_release() {
    echo -e "${YELLOW}Building project in release mode...${NC}"
    cargo build --release
    echo -e "${GREEN}✓ Build complete${NC}"
}

# Run specific benchmark group
run_benchmark_group() {
    local group=$1
    echo -e "${BLUE}Running benchmark group: ${group}${NC}"
    cargo bench --bench database_benchmark -- ${group}
}

# Run all benchmarks
run_all_benchmarks() {
    echo -e "${YELLOW}Running all benchmarks...${NC}"
    
    # Define benchmark groups
    BENCHMARK_GROUPS=(
        "single_document_insert"
        "batch_insert"
        "search"
        "simd_operations"
        "transactions"
        "concurrent"
        "throughput"
    )
    
    # Run each benchmark group
    for group in "${BENCHMARK_GROUPS[@]}"; do
        echo ""
        echo -e "${BLUE}═══════════════════════════════════════════${NC}"
        run_benchmark_group ${group}
    done
}

# Generate report
generate_report() {
    echo -e "${YELLOW}Generating benchmark report...${NC}"
    
    {
        echo "DRUSDENX BENCHMARK REPORT"
        echo "========================="
        echo "Date: $(date)"
        echo "System: $(uname -a)"
        echo "CPU: $(lscpu | grep 'Model name' | sed 's/Model name:[ ]*//')"
        echo "Memory: $(free -h | grep 'Mem:' | awk '{print $2}')"
        echo ""
        echo "Benchmark Results"
        echo "-----------------"
        
        # Extract key metrics from criterion output
        if [ -d "${BENCHMARK_DIR}" ]; then
            find ${BENCHMARK_DIR} -name "estimates.json" | while read -r file; do
                echo ""
                echo "Benchmark: $(dirname ${file} | xargs basename)"
                cat ${file} | python3 -m json.tool | grep -E '"mean"|"median"' || true
            done
        fi
    } > ${REPORT_FILE}
    
    echo -e "${GREEN}✓ Report generated: ${REPORT_FILE}${NC}"
}

# Compare with baseline
compare_baseline() {
    echo -e "${YELLOW}Comparing with baseline...${NC}"
    
    if [ -d "${BENCHMARK_DIR}" ]; then
        cargo bench --bench database_benchmark -- --baseline main || true
    else
        echo -e "${YELLOW}No baseline found. This run will be the baseline.${NC}"
    fi
}

# Print summary
print_summary() {
    echo ""
    echo -e "${GREEN}╔═══════════════════════════════════════════════════════╗${NC}"
    echo -e "${GREEN}║                 BENCHMARK COMPLETE                    ║${NC}"
    echo -e "${GREEN}╚═══════════════════════════════════════════════════════╝${NC}"
    echo ""
    echo -e "${BLUE}Results saved to:${NC}"
    echo "  • HTML Report: ${BENCHMARK_DIR}/report/index.html"
    echo "  • Text Report: ${REPORT_FILE}"
    echo ""
    echo -e "${YELLOW}To view HTML report, open:${NC}"
    echo "  file://$(pwd)/${BENCHMARK_DIR}/report/index.html"
}

# Interactive menu
show_menu() {
    echo -e "${BLUE}Select benchmark option:${NC}"
    echo "  1) Run all benchmarks"
    echo "  2) Run single insert benchmark"
    echo "  3) Run batch insert benchmark"
    echo "  4) Run search benchmark"
    echo "  5) Run SIMD operations benchmark"
    echo "  6) Run transaction benchmark"
    echo "  7) Run concurrent operations benchmark"
    echo "  8) Run throughput benchmark"
    echo "  9) Compare with baseline"
    echo "  0) Exit"
    
    read -p "Enter choice [0-9]: " choice
    
    case ${choice} in
        1) run_all_benchmarks ;;
        2) run_benchmark_group "single_document_insert" ;;
        3) run_benchmark_group "batch_insert" ;;
        4) run_benchmark_group "search" ;;
        5) run_benchmark_group "simd_operations" ;;
        6) run_benchmark_group "transactions" ;;
        7) run_benchmark_group "concurrent" ;;
        8) run_benchmark_group "throughput" ;;
        9) compare_baseline ;;
        0) exit 0 ;;
        *) echo -e "${RED}Invalid option${NC}" && show_menu ;;
    esac
}

# Main execution
main() {
    print_banner
    check_prerequisites
    
    # Parse command line arguments
    if [ $# -eq 0 ]; then
        # Interactive mode
        clean_previous
        build_release
        show_menu
        generate_report
    elif [ "$1" == "--all" ]; then
        # Run all benchmarks
        clean_previous
        build_release
        run_all_benchmarks
        generate_report
    elif [ "$1" == "--quick" ]; then
        # Quick benchmark (reduced samples)
        clean_previous
        build_release
        echo -e "${YELLOW}Running quick benchmark...${NC}"
        cargo bench --bench database_benchmark -- --sample-size 10 --measurement-time 5
        generate_report
    elif [ "$1" == "--compare" ]; then
        # Compare with baseline
        build_release
        compare_baseline
    elif [ "$1" == "--help" ]; then
        echo "Usage: $0 [OPTIONS]"
        echo ""
        echo "Options:"
        echo "  --all      Run all benchmarks"
        echo "  --quick    Run quick benchmark with reduced samples"
        echo "  --compare  Compare with baseline"
        echo "  --help     Show this help message"
        echo ""
        echo "Without options, runs in interactive mode"
    else
        echo -e "${RED}Unknown option: $1${NC}"
        echo "Use --help for usage information"
        exit 1
    fi
    
    print_summary
}

# Run main function
main "$@"
