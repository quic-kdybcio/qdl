#!/bin/sh
# SPDX-License-Identifier: BSD-3-Clause

# Build the Rust half of qdl and copy the static library and the generated
# C header to the paths meson asks for. Invoked from meson/meson.build.
#
# usage: build_rust.sh CARGO MANIFEST TARGET_DIR LIB_OUT HEADER_OUT

set -eu

# cargo runs build.rs from the crate directory, so every path handed to it
# has to be absolute.
abspath() {
	case "$1" in
	/* | [A-Za-z]:[\\/]*) printf '%s\n' "$1" ;;
	*) printf '%s/%s\n' "$(pwd)" "$1" ;;
	esac
}

cargo=$1
manifest=$(abspath "$2")
target_dir=$(abspath "$3")
lib_out=$(abspath "$4")
header_out=$(abspath "$5")

QDL_RS_HEADER=$header_out "$cargo" build --release --locked \
	--manifest-path "$manifest" --target-dir "$target_dir"

cp "$target_dir/release/libqdl_rs.a" "$lib_out"
