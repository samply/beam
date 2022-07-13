![Logo](./doc/Logo.svg) <!-- TODO: New Logo -->

Samply.Beam is a distributed task broker designed for efficient communication across strict network environments. It provides most commonly used communication patterns across strict network boundaries, end-to-end encryption and signatures, as well as certificate management and validation on top of an easy to use REST API.

## Table of Content
<!-- - [Features](#features) -->
 - [System Architecture](#system-architecture)
 - [Getting Started](#getting-started)
 - [JSON Data Objects](#data-objects-json)
 - [API Description](#api)
 - [Roadmap](#roadmap)
 
 ## Why use Samply.Beam?
Samply.Beam was developed to solve a principal difficulty of interconnecting federated applications across restrictive network boundaries. Any federated data computation requires some form of communication among the nodes, often in a reliable and high-performance manner. However, in high-security environments such as internal hospital networks, this communication is severely restricted, e.g., by strict firewall rules, forbidding inbound connections and/or using exotic combinations of HTTP proxy servers. Many currently employed solutions place high technical and organizational burdens on each participating site (e.g., message queues requiring servers in a DMZ) or are even considered harmful to the network's security (e.g., VPN overlay networks), suffer from performance issues and introduce additional complexity to the system. 

We developed Samply.Beam as a reusable, easy to maintain, secure, high-performance communication layer allowing us to handle most common communication patterns in distributed computation in an efficient and reusable way, while removing complexity from the applications. Samply.Beam handles all "plumbing", such as the negotiation of communication parameters, target discovery, and helps with routinely performed tasks such as authentication and authorization, end-to-end encryption and signatures, and certificate management and validation. This way your application can focus on its main purpose, without getting bogged down by integration tasks. Samply.Beam was created as the latest iteration of the [Bridgehead](https://github.com/samply/bridgehead)'s communication layer, but the software is fully content-agnostic: Only your applications have to understand the communication payload. This allows the integration of arbitrary applications in a Samply.Beam federation.

<!-- TODO, merge with text above
## Features

 - Made for strict network environment in University Hospitals:
   - Highly performant even with exotic proxy and firewall systems
   - No DMZ, ..., required
   - Covers all common connection patters: Point-to-Point, Fan-Out, Fan-In, Queues, ...
 - End-to-End security by using AES-GCM encryption and digital signatures
 - Local component for easy integration: Handles all "plumbing", such as authentication, network issues, ...
 - Easily extensible: Content agnostic, open REST interface, simple to use, simple to adapt
-->

## System Architecture

![Architecture Schema](./doc/Architecture.svg) <!-- TODO: Update and remove margin at top -->

*Samply.Beam* consists of two centrally run components and one proxy at each distributed node. The *Samply.Broker* is the central component responsible for facilitating connections, storing and forwarding tasks and messages, and communication with the central *Certificate Authority*, a [Hashicorp Vault](https://github.com/hashicorp/vault) instance managing all certificates required for signing and encrypting the payload. The local *Samply.Proxy* handles all communication with the broker, as well as authentication, encryption and signatures.

Each component in the system is uniquely identified by its hierarchical *BeamId*:
```
app123.proxy345.broker3.samply.de
<------------------------------->
            AppId
      <------------------------->
                ProxyId
                <--------------->
                      BrokerId
```
This way each component, mainly applications but Proxies and Brokers as well,
can be addressed in tasks and further federation of brokers is transparently
possible.

## Getting started
Running the `central` binary will open a central broker instance listening on `0.0.0.0:8080`. The instance can be queried via the API (see next section).

## Data objects (JSON)
### Task
Tasks are represented in the following structure:

```json
{
  "id": "70c0aa90-bfcf-4312-a6af-42cbd57dc0b8",
  "from": "app7.proxy-hd.broker-project1.samply.de",
  "to": [
    "app1.proxy-hd.broker-project1.samply.de",
    "app5.proxy-ma.broker-project1.samply.de"
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
- `from`: BeamID of the submitting applications. Is automatically set by the Proxy according to the authentification info.
- `to`: BeamIDs of *workers* allowed to retrieve the task and submit results.
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
  "from": "app1.proxy-hd.broker-project1.samply.de",
  "to": [
    "app7.proxy-hd.broker-project1.samply.de"
  ],
  "task": "70c0aa90-bfcf-4312-a6af-42cbd57dc0b8",
  "result": {
    "succeeded": "Successfully quenched 1.43e14 flux pulse devices"
  }
}
```

A failed task:
```json
{
  "id": "24a49494-6a00-415f-80fc-b2ae34658b98",
  "from": "app5.proxy-ma.broker-project1.samply.de",
  "to": [
    "app7.proxy-hd.broker-project1.samply.de"
  ],
  "task": "70c0aa90-bfcf-4312-a6af-42cbd57dc0b8",
  "result": {
    "permfailed": "Unable to decrypt quantum state"
  }
}
```

- `id`: UUID identifying the result. When the result is initially created, the value is ignored and replaced by a server-generated one.
- `from`: BeamID identifying the client submitting this result. This needs to match an entry the `to` field in the task.
- `to`: BeamIDs the intended recipients of the result. Used for encrypted payloads.
- `task`: UUID identifying the task this result belongs to.
- `result`: Defines status of this work result. Possible values `unclaimed`, `tempfailed(body)`, `permfailed(body)`, `succeeded(body)`.
- `result.body`: Either carries the actual result payload of the task (`succeeded`) or an error message.

## API
### Create task
Create a new task to be worked on by defined workers.

Method: `POST`  
URL: `/v1/tasks`  
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
URL: `/v1/tasks`  
Parameters:
- `to` (optional): Fetch only tasks directed to this worker.
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
URL: `/v1/tasks/<task_id>/results`  
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

### Long-polling API access
As part of making this API performant, all reading endpoints support long-polling as an efficient alternative to regular (repeated) polling. Using this function requires the following parameters:
- `poll_count`: The API call will block until this many results are available ...
- `poll_timeout`: ... or this many milliseconds have passed, whichever comes first.

For example, retrieving a task's results:
- `GET /v1/tasks/<task_id>/results` will return immediately with however many results are available,
- `GET /v1/tasks/<task_id>/results?poll_count=5` will block forever until 5 results are available,
- `GET /v1/tasks/<task_id>/results?poll_count=5&poll_timeout=30000` will block until 5 results are available or 30 seconds have passed (whichever comes first). In the latter case, HTTP code 206 (Partial Content) is returned to indicate that the result is incomplete.

## Roadmap
- [X] API Key authentication of local applications
- [X] Certificate management
- [X] End-to-End signatures
- [ ] End-to-End encryptions
- [ ] Docker deployment packages
- [ ] Integration of OAuth2
- [ ] Integration of LDAP
