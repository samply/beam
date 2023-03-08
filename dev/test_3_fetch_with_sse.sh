#!/bin/bash -e

testing "Fetch existing task with proxy1 (via SSE)"

RET=$(curl_get_sse $APP1_P1 -v $P1/v1/tasks/70c0aa90-bfcf-4312-a6af-42cbd57dc0b8/results)
CODE=$(echo $RET | jq -r .response_code)
BODY=$(echo $RET | jq -r .body)

if [ "$CODE" != "200" ]; then
    fail "$RET" Unable to fetch the existing task as app1.proxy1 via SSE
fi

EVENT_LINES=$(echo "$BODY" | grep '^event:' | sed 's/^event://g')
COUNT=0
IFS=$'\n'
for LINE in $EVENT_LINES; do
    if [ "$LINE" != "new_result" ]; then
        fail "$RET" Got unexpected SSE event: $LINE
    fi
    COUNT=$((COUNT+1))
done
if [ $COUNT -ne 2 ]; then
    fail "$RET" Expected to see 2 SSE event results, got $COUNT.
fi

DATA_LINES=$(echo "$BODY" | grep '^data:' | grep '70c0aa90-bfcf-4312-a6af-42cbd57dc0b8' | sed 's/^data://g')
COUNT=0
IFS=$'\n'
for LINE in $DATA_LINES; do
    echo "Line: $LINE"
    TASK=$(echo "$LINE" | jq -r .task)
    echo "Task: $TASK"
    if [ "$TASK" != "70c0aa90-bfcf-4312-a6af-42cbd57dc0b8" ]; then
        fail "$RET" Got unexpected SSE data line: $LINE
    fi
    COUNT=$((COUNT+1))
done

if [ $COUNT -ne 2 ]; then
    fail "$RET" Expected to see 2 SSE results, got $COUNT.
fi

success

testing "Fetch existing task with proxy2 (via SSE)"

RET=$(curl_get_sse $APP1_P2 -v $P2/v1/tasks/70c0aa90-bfcf-4312-a6af-42cbd57dc0b8/results)
CODE=$(echo $RET | jq -r .response_code)
BODY=$(echo $RET | jq -r .body)

if [ "$CODE" != "401" ]; then
    fail "$RET" Trying to fetch a task not belonging to me, I expected 401, got $CODE
fi

if [ "$BODY" != "Not your task." ]; then
    fail "$RET" Trying to fetch a task not belonging to me, I expected body \"Not your task.\", got \"$body\"
fi

success

