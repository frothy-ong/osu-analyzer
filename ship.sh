#!/bin/sh
git add -A
git commit -q -m "${1:-update}" || true
git push
