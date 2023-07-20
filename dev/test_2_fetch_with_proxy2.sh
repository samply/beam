#!/bin/bash -e

testing Fetch existing task with proxy1

RET=$(curl_get $APP1_P1 -v $P1/v1/tasks/70c0aa90-bfcf-4312-a6af-42cbd57dc0b8/results)
CODE=$(echo $RET | jq -r .response_code)
BODY=$(echo $RET | jq -r .body)
echo $BODY

if [ "$CODE" != "200" ]; then
    fail "$RET" Unable to fetch the existing task as app1.proxy1
fi

success

testing Fetch existing task with proxy2

RET=$(curl_get $APP1_P2 -v $P2/v1/tasks/70c0aa90-bfcf-4312-a6af-42cbd57dc0b8/results)
CODE=$(echo $RET | jq -r .response_code)
BODY=$(echo $RET | jq -r .body)

if [ "$CODE" != "401" ]; then
    fail "$RET" Trying to fetch a task not belonging to me, I expected 401, got $CODE
fi

success

