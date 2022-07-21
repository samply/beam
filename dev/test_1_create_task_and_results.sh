#!/bin/bash -e

testing Create task

RET=$(echo "${TASK0}" | curl_post -v $P1/v1/tasks)
CODE=$(echo $RET | jq -r .response_code)

if [ "$CODE" != "201" ]; then
    fail "$RET" "Expected code 201, got $CODE"
fi

success

testing Deliver result \#1

RET=$(echo "${RESULT_BY_APP1}" | curl_post -v $P1/v1/tasks/70c0aa90-bfcf-4312-a6af-42cbd57dc0b8/results)
CODE=$(echo $RET | jq -r .response_code)

if [ "$CODE" != "201" ]; then
    fail "$RET" Expected code 201, got $CODE
fi

success

testing Deliver result \#2
RET=$(echo "${RESULT_BY_APP2}" | curl_post -v $P1/v1/tasks/70c0aa90-bfcf-4312-a6af-42cbd57dc0b8/results)
CODE=$(echo $RET | jq -r .response_code)

if [ "$CODE" != "401" ]; then
    fail "$RET" We delivered as app2 with an auth of app1 -- Expected code 401, got $CODE
fi

success
