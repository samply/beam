#!/bin/bash -e

source ./dev/beamdev noop

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

dd if=/dev/zero bs=50M status=none count=1 | sed "s/\x0/A/g" >> $TASK

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
