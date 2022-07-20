#!/bin/bash -e

testing Deliver result \#1

CODE=$(echo "${RESULT_BY_APP1}" | curl_post_out response_code $P/tasks/70c0aa90-bfcf-4312-a6af-42cbd57dc0b8/results)

if [ "$CODE" != "201" ]; then
    fail Expected code 201, got $CODE
fi

success

testing Deliver result \#2
CODE=$(echo "${RESULT_BY_APP2}" | curl_post_out response_code $P/tasks/70c0aa90-bfcf-4312-a6af-42cbd57dc0b8/results)

if [ "$CODE" != "201" ]; then
    fail Expected code 201, got $CODE
fi

success
