#!/bin/bash -e

testing Create very large task

TASK=$(mktemp)
echo '
{
  "id": "70c0aa90-bfcf-4312-a6af-42cbd57dc0b9",
  "from": "app1.proxy1.broker",
  "to": [
    "app1.proxy1.broker",
    "app2.proxy1.broker"
  ],
  "body": "' >> $TASK

# With a jwt size limit of 100MB the max size for a message body is about 55MB
# because the encrypted body gets base64 encoded and the message gets base64 encoded again
# when converted to a jwt when being sent to the broker.
# Each resulting in a 1.33 fold increase in size, which leads to a roughly 1.75 fold size increase for each message
dd if=/dev/zero bs=55M status=none count=1 | sed "s/\x0/A/g" >> $TASK

echo '",
  "failure_strategy": {
    "retry": {
      "backoff_millisecs": 1000,
      "max_tries": 5
    }
  },
  "ttl": "1m",
  "metadata": "The broker can read and use this field e.g., to apply filters on behalf of an app"
}' >> $TASK

RET=$(curl_post_file $APP1_P1 $TASK -v $P1/v1/tasks)
rm $TASK
CODE=$(echo $RET | jq -r .response_code)

if [ "$CODE" != "201" ]; then
    fail "$RET" "Expected code 201, got $CODE"
fi

success

