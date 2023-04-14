#!/bin/bash -e

testing Create very large task

# RET=$(curl_post_file $APP1_P1 very_large_task.json -v $P1/v1/tasks)
# CODE=$(echo $RET | jq -r .response_code)

# if [ "$CODE" != "201" ]; then
#     fail "$RET" "Expected code 201, got $CODE"
# fi

success

