# Samply.Beam
Samply.Beam is a distributed task broker designed for efficient communication across strict network environments. Using an easy to use REST interface, it provides most commonly used communication patterns across strict network boundaries, end-to-end encryption and signatures, as well as certificate management and validation.

## Why use Samply.Beam?
Samply.Beam was developed to solve a principal difficulty of interconnecting apllications across strict network boundaries, e.g. in hospital IT environments.
For data protection reasions (and good reasons, at that) the applications' ability to freely communicate are severely restricted, e.g., by strict firewall rules, forbidding inbound connections and/or using HTTP proxy systems. Howover, for federated data analysis these applications must be able to perform high-performance communication. Many currently employed solutions, such as placing a message queue in a not-as-tighly-controlled network environment (a DMZ) and let the application in the "high-security" network poll that queue, suffer from performance issues and introduce many complexities to the system. Many additional tasks, user management, authentication and authorization, and so on, are handled by the applications.

After years of suffering basically the same difficulties, we developed Samply.Beam as an easy to deploy, secure, high-performance "communication helper" allowing us to handle most common communication pattern in distributed computation in an efficient and simple way, while removing complexity from the applications. Samply.Beam handles all "plumbing", such as the negotiation of communication parameters, target discovery, but helps with routiniely performed tasks such as authentication and authorization, end-to-end encryption and signatures, and certificate management and validation. This way your application can focus on it's main purpose, without getting boged down by integration tasks.

## Table of Content


## System Architecture

![Architecture Schema](./doc/Architecture.svg)

Samply.Beam consists of a centrally run component and distributed nodes. 

## Getting started
Running the `central` binary will open a central broker instance listening on `0.0.0.0:8080`. The instance can be queried via the API (see next section).

## Data objects (JSON)
### Task
Tasks are represented in the following structure:

```json
{
  "id": "70c0aa90-bfcf-4312-a6af-42cbd57dc0b8",
  "to": [
    "6e3cf893-c134-45d2-b9f3-b02d92ad51e0",
    "0abd8445-b4a9-4e20-8a4a-bd97ed57745c"
  ],
  "task_type": "My important task",
  "body": "Much work to do",
  "failure_strategy": {
    "retry": {
      "backoff_millisecs": 1000,
      "max_tries": 5
    }
  }
}
```

- `id`: UUID to identify the task. When the task is initially created, the value is ignored and replaced by a server-generated one.
- `to`: UUIDs of *workers* allowed to retrieve the task and submit results.
- `task_type`: Well-known identifier for the type of work. Not interpreted by the Broker.
- `body`: Description of work to be done. Not interpreted by the Broker.
- `failure_strategy`: Advises each client how to handle failures. Possible values `discard`, `retry`.
- `failure_strategy.retry`: How often to retry (`max_tries`) a failed task and how long to wait in between each try (`backoff_millisecs`).

### Result
Each task can hold 0...n results by each *worker* defined in the task's `to` field.

A succeeded result for the above task:
```json
{
  "id": "8db76400-e2d9-4d9d-881f-f073336338c1",
  "worker_id": "6e3cf893-c134-45d2-b9f3-b02d92ad51e0",
  "task": "70c0aa90-bfcf-4312-a6af-42cbd57dc0b8",
  "result": {
    "succeeded": "<result payload>"
  }
}
```

A failed task:
```json
{
  "id": "24a49494-6a00-415f-80fc-b2ae34658b98",
  "worker_id": "0abd8445-b4a9-4e20-8a4a-bd97ed57745c",
  "task": "70c0aa90-bfcf-4312-a6af-42cbd57dc0b8",
  "result": {
    "permfailed": "Unable to decrypt quantum state"
  }
}
```

- `id`: UUID identifying the result. When the result is initially created, the value is ignored and replaced by a server-generated one.
- `worker_id`: UUID identifying the client submitting this result. This needs to match an entry the `to` field in the task.
- `task`: UUID identifying the task this result belongs to.
- `result`: Defines status of this work result. Possible values `unclaimed`, `tempfailed(body)`, `permfailed(body)`, `succeeded(body)`.
- `result.body`: Either carries the actual result payload of the task (`succeeded`) or an error message.

## API
### Create task
Create a new task to be worked on by defined workers.

Method: `POST`  
URL: `/tasks`  
Body: see [Task](#task)  
Parameters: none

Returns:
```
HTTP/1.1 201 Created
location: /tasks/b999cf15-3c31-408f-a3e6-a47502308799
content-length: 0
date: Mon, 27 Jun 2022 13:58:35 GMT
```

In subsequent requests, use the URL defined in the `location` header to refer to the task (NOT the one you supplied in your POST body).

### Retrieve task
Workers regularly call this endpoint to retrieve submitted tasks.

Method: `GET`  
URL: `/tasks`  
Parameters:
- `worker_id` (optional): Fetch only tasks directed to this worker.
- [long polling](#long-polling) is supported.

Returns an array of tasks, cf. [here](#task)
```
HTTP/1.1 200 OK
content-type: application/json
content-length: 220
date: Mon, 27 Jun 2022 14:05:59 GMT

[
  {
    "id": ...
  }
)
```

### Retrieve results
The submitter of the task (see [Create Task](#create-task)) calls this endpoint to retrieve the results.

Method: `GET`  
URL: `/tasks/<task_id>/results`  
Parameters:
- [long polling](#long-polling) is supported.

Returns an array of results, cf. [here](#result)
```
HTTP/1.1 200 OK
content-type: application/json
content-length: 179
date: Mon, 27 Jun 2022 14:26:45 GMT

[
  {
    "id": ...
  }
]
```

## Long-polling API access
As part of making this API performant, all reading endpoints support long-polling as an efficient alternative to regular (repeated) polling. Using this function requires the following parameters:
- `poll_count`: The API call will block until this many results are available ...
- `poll_timeout`: ... or this many milliseconds have passed, whichever comes first.

For example, retrieving a task's results:
- `GET /tasks/<task_id>/results` will return immediately with however many results are available,
- `GET /tasks/<task_id>/results?poll_count=5` will block forever until 5 results are available,
- `GET /tasks/<task_id>/results?poll_count=5&poll_timeout=30000` will block until 5 results are available or 30 seconds have passed (whichever comes first). In the latter case, HTTP code 206 (Partial Content) is returned to indicate that the result is incomplete.

## Roadmap
- [X] API Key authentication of local applications
- [X] Certificate management
- [X] End-to-End signatures
- [ ] End-to-End encryptions
- [ ] Docker deployment packages
- [ ] Integration of OAuth2
- [ ] Integration of LDAP