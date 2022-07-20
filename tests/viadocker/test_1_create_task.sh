#!/bin/bash -e

testing Create task
CODE=$(echo "${TASK0}" | curl_post_out response_code $P/tasks)

if [ "$CODE" != "201" ]; then
    fail Expected code 201, got $CODE
fi

success
