#!/bin/bash -e

testing Create task TASK_BY_A1P1_FOR_A1P2

RET=$(echo $TASK_BY_A1P1_FOR_A1P2 | curl_post $APP1_P1 -v $P1/v1/tasks)
CODE=$(echo $RET | jq -r .response_code)

if [ "$CODE" != "201" ]; then
    fail "$RET" Unable to create task. Expected 201, got $CODE
fi

success

testing Fetch that task with A1P2

RET=$(curl_get $APP1_P2 -v $P2/v1/tasks?filter=todo)
CODE=$(echo $RET | jq -r .response_code)
BODY=$(echo $RET | jq -r .body)

if [ "$CODE" != "200" ]; then
    fail "$RET" Failed to fetch task. Expected 200, got $CODE
fi

if [ "$(echo $BODY | jq --sort-keys -c .[0])" != "$(echo $TASK_BY_A1P1_FOR_A1P2 | jq --sort-keys -c)" ]; then
    fail "$RET" I got a task but not the right one? I expected body \"$TASK_BY_A1P1_FOR_A1P2\", got \"$BODY\"
fi

if [ "$(echo $BODY | jq --sort-keys -c .[1])" != "null" ]; then
    fail "$RET" I got back too many tasks. Expected 1, got at least 2.
fi

success
