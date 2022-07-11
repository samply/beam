#!/bin/bash -e

SD=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )

source $SD/setup.sh

#start

for test in tests/scripts/test_*.sh; do 
    echo "Testing $(basename $test) ..."
    bash $test
done

#stop
#clean