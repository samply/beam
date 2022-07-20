#!/bin/bash -e

SD=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )

cd $SD

source beamdev noop

build
start_bg

for test in test_*.sh; do
    echo "Testing $(basename $test) ..."
    bash $test
done

clean
