#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "$0")/.." && pwd)"
test_dir="$(mktemp -d -t dac-wake-test.XXXXXX)"
trap 'rm -rf "$test_dir"' EXIT
module_cache="$test_dir/swift-module-cache"
mkdir -p "$module_cache"

swiftc \
  "$project_dir/native/audio-host/AudioHost.swift" \
  -module-cache-path "$module_cache" \
  -framework AppKit \
  -framework AVFoundation \
  -framework Speech \
  -o "$test_dir/audio-host-test"

"$test_dir/audio-host-test" --self-test-wake
