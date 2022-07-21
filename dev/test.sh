#!/bin/bash -e

SD=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )

cd $SD

source beamdev noop

function start() {
  trap "echo; echo; clean" EXIT
  start_bg
}

function test() {
  for test in test_*.sh; do
    echo "======="
    echo "=> $(basename $test) ..."
    source $test
  done
}

function build() {
  if [ ! -e $SD/../../artifacts/binaries-amd64/proxy ]; then
    build_rust
  fi
  build_docker
}

case "$1" in
  start)
    start
    ;;
  test)
    test
    ;;
  ci)
    build
    start
    test
    ;;
  *)
    echo "Usage: $0 start|test|ci"
    exit 1
    ;;
esac
