#!/bin/bash -e

SD=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )

source $SD/setup.sh

cd $SD/../

exec cargo test -- --test-threads=1
