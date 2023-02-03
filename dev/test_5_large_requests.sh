#!/bin/bash -e

testing Create large task

RET=$(curl_post_file $APP1_P1 large_task.json -v $P1/v1/tasks)
CODE=$(echo $RET | jq -r .response_code)

if [ "$CODE" != "201" ]; then
    fail "$RET" "Expected code 201, got $CODE"
fi

success

#testing Deliver large result \#1
#
#RET=$(echo "${LARGE_RESULT_BY_A1P1}" | curl_put $APP1_P1 -v $P1/v1/tasks/70c0ca90-bfcf-4312-a6af-42cbd57dc0b8/results/app1.proxy1.broker)
#CODE=$(echo $RET | jq -r .response_code)
#
#if [ "$CODE" != "201" ]; then
    #fail "$RET" Expected code 201, got $CODE
#fi
#
#success
