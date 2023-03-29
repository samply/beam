#!/bin/bash -e

testing Create task

TASK0="$(echo "$TASK0" | task_with_ttl 10s "$TASK0")"
RET=$(echo "${TASK0}" | curl_post $APP1_P1 -v $P1/v1/tasks)
CODE=$(echo $RET | jq -r .response_code)

if [ "$CODE" != "201" ]; then
    fail "$RET" "Expected code 201, got $CODE"
fi

success

testing Deliver result \#1

RET=$(echo "${RESULT_BY_APP1}" | curl_put $APP1_P1 -v $P1/v1/tasks/70c0aa90-bfcf-4312-a6af-42cbd57dc0b8/results/app1.proxy1.broker)
CODE=$(echo $RET | jq -r .response_code)

if [ "$CODE" != "201" ]; then
    fail "$RET" Expected code 201, got $CODE
fi

success

testing Deliver result \#2 as wrong app
RET=$(echo "${RESULT_BY_APP2}" | curl_put $APP1_P1 -v $P1/v1/tasks/70c0aa90-bfcf-4312-a6af-42cbd57dc0b8/results/app1.proxy1.broker)
CODE=$(echo $RET | jq -r .response_code)

if [ "$CODE" != "401" ]; then
    fail "$RET" We delivered as app2 with an auth of app1 -- Expected code 401, got $CODE
fi

success

testing Deliver result \#2 as correct app
RET=$(echo "${RESULT_BY_APP2}" | curl_put $APP2_P1 -v $P1/v1/tasks/70c0aa90-bfcf-4312-a6af-42cbd57dc0b8/results/app2.proxy1.broker)
CODE=$(echo $RET | jq -r .response_code)

if [ "$CODE" != "201" ]; then
    fail "$RET" We delivered correctly as app2 -- Expected code 201, got $CODE
fi

success

