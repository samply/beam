#!/bin/bash

export TASK0='
{
  "id": "70c0aa90-bfcf-4312-a6af-42cbd57dc0b8",
  "from": "app1.proxy1.broker",
  "to": [
    "app1.proxy1.broker",
    "app2.proxy1.broker"
  ],
  "body": "Much work to do",
  "failure_strategy": {
    "retry": {
      "backoff_millisecs": 1000,
      "max_tries": 5
    }
  },
  "ttl": [TTL],
  "metadata": "The broker can read and use this field e.g., to apply filters on behalf of an app"
}'

export RESULT_BY_APP1='
{
  "from": "app1.proxy1.broker",
  "to": [
    "app1.proxy1.broker"
  ],
  "task": "70c0aa90-bfcf-4312-a6af-42cbd57dc0b8",
  "status": "succeeded",
  "body": "Successfully quenched 1.43e14 flux pulse devices",
  "metadata": ["Arbitrary", "types", "are", "possible"]
}'

export RESULT_BY_APP2='
{
  "from": "app2.proxy1.broker",
  "to": [
    "app1.proxy1.broker"
  ],
  "task": "70c0aa90-bfcf-4312-a6af-42cbd57dc0b8",
  "status": "permfailed",
  "body": "Unable to decrypt quantum state",
  "metadata": {
    "complex": "A map (key 'complex') is possible, too"
  }
}'

export TASK_BY_A1P1_FOR_A1P2='
{
  "id": "dfa0dbe1-46f7-42da-b3f5-7b13dc92caae",
  "from": "app1.proxy1.broker",
  "to": [
    "app1.proxy2.broker"
  ],
  "body": "So much work!",
  "failure_strategy": {
    "retry": {
      "backoff_millisecs": 1000,
      "max_tries": 5
    }
  },
  "ttl": [TTL],
  "metadata": null
}'
export TASK2_BY_A1P1_FOR_A1P2='
{
  "id": "6f531223-3699-4f6e-b7bf-88d8064fea7e",
  "from": "app1.proxy1.broker",
  "to": [
    "app1.proxy2.broker"
  ],
  "body": "So much work!",
  "failure_strategy": {
    "retry": {
      "backoff_millisecs": 1000,
      "max_tries": 5
    }
  },
  "ttl": [TTL],
  "metadata": null
}'
export RESULT_BY_APP1_P2='
{
  "from": "app1.proxy2.broker",
  "to": [
    "app1.proxy1.broker"
  ],
  "task": "6f531223-3699-4f6e-b7bf-88d8064fea7e",
  "status": "claimed",
  "metadata": {
    "complex": "A map (key 'complex') is possible, too"
  }
}'
