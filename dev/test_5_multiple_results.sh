#!/bin/bash -e

testing Create task for all proxies

TASK_ALL="$(echo "$TASK_BY_A1P1_FOR_A1PALL" | task_with_ttl 3600 "$TASK_BY_A1P1_FOR_A1PALL")"
RET=$(echo "${TASK_ALL}" | curl_post $APP1_P1 -v $P1/v1/tasks)
CODE=$(echo $RET | jq -r .response_code)

if [ "$CODE" != "201" ]; then
    fail "$RET" "Expected code 201, got $CODE"
fi

success

testing Deliver result claim \#1

RET=$(echo "${MULTI_CLAIM_BY_A1P1}" | curl_put $APP1_P1 -v $P1/v1/tasks/dfa0dbe1-46f7-32da-b3f5-7b13dc92caae/results/app1.proxy1.broker)
CODE=$(echo $RET | jq -r .response_code)

if [ "$CODE" != "201" ]; then
    fail "$RET" Expected code 201, got $CODE
fi

success

testing Deliver result \#2

RET=$(echo "${MULTI_RESULT_BY_A1P2}" | curl_put $APP1_P2 -v $P2/v1/tasks/dfa0dbe1-46f7-32da-b3f5-7b13dc92caae/results/app1.proxy2.broker)
CODE=$(echo $RET | jq -r .response_code)

if [ "$CODE" != "201" ]; then
    fail "$RET" Expected code 201, got $CODE
fi

success

testing Deliver real result for claim \#1

RET=$(echo "${MULTIPLE_RESULT_BY_A1P1}" | curl_put $APP1_P1 -v $P1/v1/tasks/dfa0dbe1-46f7-32da-b3f5-7b13dc92caae/results/app1.proxy1.broker)
CODE=$(echo $RET | jq -r .response_code)

if [ "$CODE" != "201" ]; then
    fail "$RET" Expected code 201, got $CODE
fi

success

testing Deliver result \#3

RET=$(echo "${MULTI_RESULT_BY_A1P3}" | curl_put $APP1_P3 -v $P3/v1/tasks/dfa0dbe1-46f7-32da-b3f5-7b13dc92caae/results/app1.proxy3.broker)
CODE=$(echo $RET | jq -r .response_code)

if [ "$CODE" != "201" ]; then
    fail "$RET" Expected code 201, got $CODE
fi

success
