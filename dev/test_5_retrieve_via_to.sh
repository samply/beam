#!/bin/bash -e

testing All tasks should be expired

RET=$(curl_get $APP1_P2 -v $P2/v1/tasks?to=$APP1_P2)
CODE=$(echo $RET | jq -r .response_code)
BODY=$(echo $RET | jq -r .body)

if [ "$CODE" != "200" ] && [ "$(echo $BODY | jq --sort-keys -c .[0])" != "null" ]; then
    fail "$RET" Failed Query or task queue not empty. Expected 200 with Body [], got $CODE with $BODY
fi

success

testing Create task TASK2_BY_A1P1_FOR_A1P2

TASK2_BY_A1P1_FOR_A1P2="$(echo "$TASK2_BY_A1P1_FOR_A1P2" | task_with_ttl 4s)"
RET=$(echo $TASK2_BY_A1P1_FOR_A1P2 | curl_post $APP1_P1 -v $P1/v1/tasks)
CODE=$(echo $RET | jq -r .response_code)

if [ "$CODE" != "201" ]; then
    fail "$RET" Unable to create task. Expected 201, got $CODE
fi

success

testing Fetch that task with A1P2

RET=$(curl_get $APP1_P2 -v $P2/v1/tasks?to=$APP1_P2)
CODE=$(echo $RET | jq -r .response_code)
BODY=$(echo $RET | jq -r .body)

if [ "$CODE" != "200" ]; then
    fail "$RET" Failed to fetch task. Expected 200, got $CODE
fi

if [ "$(echo $BODY | jq --sort-keys -c .[1])" != "null" ]; then
    fail "$RET" I got back too many tasks. Expected 1, got at least 2.
fi

if [ "$(echo $BODY | jq --sort-keys -c '.[0] | del(.ttl)')" != "$(echo $TASK2_BY_A1P1_FOR_A1P2 | jq --sort-keys -c 'del(.ttl)')" ]; then
    fail "$RET" I got a task but not the right one? I expected body \"$TASK2_BY_A1P1_FOR_A1P2\", got \"$BODY\"
fi


success

sleep 3

testing Check that the task has correctly expired

RET=$(curl_get $APP1_P2 -v $P2/v1/tasks?to=$APP1_P2)
CODE=$(echo $RET | jq -r .response_code)
BODY=$(echo $RET | jq -r .body)

if [ "$CODE" != "200" ]; then
    fail "$RET" The task has not correctly expired. Expected 200, got $CODE and body \"$BODY\".
fi

if [ "$BODY" != "[]" ]; then
    fail "$RET" I got a task that should have been expired. Expected body \"[]\", got \"$BODY\"
fi

success

testing After expiration, we should be able to re-create the same task

TASK_BY_A1P1_FOR_A1P2="$(echo "$TASK2_BY_A1P1_FOR_A1P2" | task_with_ttl 10s)"
RET=$(echo $TASK_BY_A1P1_FOR_A1P2 | curl_post $APP1_P1 -v $P1/v1/tasks)
CODE=$(echo $RET | jq -r .response_code)

if [ "$CODE" != "201" ]; then
    fail "$RET" Unable to create task. Expected 201, got $CODE
fi

success

testing Fetch that new task with A1P2

RET=$(curl_get $APP1_P2 -v $P2/v1/tasks?to=$APP1_P2)
CODE=$(echo $RET | jq -r .response_code)
BODY=$(echo $RET | jq -r .body)

if [ "$CODE" != "200" ]; then
    fail "$RET" Failed to fetch task. Expected 200, got $CODE
fi

if [ "$(echo $BODY | jq --sort-keys -c '.[0] | del(.ttl)')" != "$(echo $TASK2_BY_A1P1_FOR_A1P2 | jq --sort-keys -c 'del(.ttl)')" ]; then
    fail "$RET" I got a task but not the right one? I expected body \"$TASK2_BY_A1P1_FOR_A1P2\", got \"$BODY\"
fi

if [ "$(echo $BODY | jq --sort-keys -c .[1])" != "null" ]; then
    fail "$RET" I got back too many tasks. Expected 1, got at least 2.
fi

success

testing Claim the newly created task

RET=$(echo "${RESULT_BY_APP1_P2}" | curl_put $APP1_P2 -v $P2/v1/tasks/6f531223-3699-4f6e-b7bf-88d8064fea7e/results/app1.proxy2.broker)
CODE=$(echo $RET | jq -r .response_code)

if [ "$CODE" != "201" ]; then
    fail "$RET" We delivered correctly as app2 -- Expected code 201, got $CODE
fi

success

sleep 4

testing Check that the claimed task has correctly expired

RET=$(curl_get $APP1_P2 -v $P2/v1/tasks?to=$APP1_P2)
CODE=$(echo $RET | jq -r .response_code)
BODY=$(echo $RET | jq -r .body)

if [ "$CODE" != "200" ]; then
    fail "$RET" The task has not correctly expired. Expected 200, got $CODE and body \"$BODY\".
fi

if [ "$BODY" != "[]" ]; then
    fail "$RET" I got a task that should have been expired. Expected body \"[]\", got \"$BODY\"
fi

success
