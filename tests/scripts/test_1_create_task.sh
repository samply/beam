#!/bin/bash -e

SD=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )

source $SD/setup.sh

#start

testing Create task
CODE=$(do_curl_noout response_code --data "${TASK0}" $P/tasks)

if [ "$CODE" != "201" ]; then
    fail Expected code 201, got $CODE
fi

success
