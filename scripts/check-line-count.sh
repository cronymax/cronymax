#!/bin/bash
set -e
violations=$(find src -name "*.rs" -exec wc -l {} + | awk '$1 > 600 && $2 != "total" {print}')
if [ -n "$violations" ]; then
  echo "ERROR: Files exceeding 600 lines:"
  echo "$violations"
  exit 1
fi
echo "All .rs files in src/ are within the 600-line limit."
