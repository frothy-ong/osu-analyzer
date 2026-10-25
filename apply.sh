#!/bin/sh
unzip -o ~/Downloads/changes.zip
rm ~/Downloads/changes.zip
cargo build 2>&1 | head -60
