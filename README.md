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
app3.proxy2.broker1.samply.de
<--------------------------->
            AppId
     <---------------------->
             ProxyId
            <--------------->
                 BrokerId
```
Although all IDs may look like fully-qualified domain names:
- Only the *BrokerId* has to be a DNS-resolvable FQDN reachable via the network (Proxies will communicate with `https://broker1.samply.de/...`)
- The *ProxyId* (`proxy2...`) is not represented in DNS but via the Proxy's certificate, which states `CN=proxy2.broker2.samply.de`
- Finally, the *AppId* (`app3...`) results from using the correct API key in communication with the Proxy (Header `Authorization: app3.broker2.samply.de <app3's API key>`)

In practice,
- there is one Broker per research network (`broker1.samply.de`)
- each site has one Bridgehead with one Proxy instance (`proxy2` for site #2)
- many apps use `proxy2` to communicate within the network (`app1`, `app2`, `app3`, ...)

This design ensures that each component, mainly applications but Proxies and Brokers as well, can be addressed in tasks. Should the need arise in the future, this network could be federated by federating the brokers (not unsimilar to E-Mail/SMTP, XMPP, etc.)

The Proxies have to fetch certificates from the central Certificate Authority, however, this communication is relayed by the broker. This ensures, that no external access to the CA is required.

## Getting started
The following paragraph simulates the creation and the completion of a task
using [cURL](http://curl.se) calls. Two parties (and their Samply.Proxies) are
connected via a central broker. Each party runs an application, called `app`.
We will simulate this application.

The used BeamIds are the following:

| System             | BeamID                       |
|--------------------|------------------------------|
| Broker             | broker.example.de            |
| Proxy1             | proxy1.broker.example.de     |
| App behind Proxy 1 | app.proxy1.broker.example.de |
| Proxy2             | proxy2.broker.example.de     |
| App behind Proxy 2 | app.proxy2.broker.example.de |

In this example, we use the same ApiKey `AppKey` for both parties.

### Creating a task
`app` at party 1 has some important work to distribute. It knows, that `app`
at party 2 is capable of solving it, so it asks `proxy1.broker.example.de` to
create that new task:
```
curl -k -v --json '{"body":"What is the answer to the ultimate question of life, the universe, and everything?","failure_strategy":{"retry":{"backoff_millisecs":1000,"max_tries":5}},"from":"app.proxy1.broker.example.de","id":"70c0aa90-bfcf-4312-a6af-42cbd57dc0b8","metadata":"The broker can read and use this field e.g., to apply filters on behalf of an app","to":["app.proxy2.broker.example.de"],"ttl":600}' -H "Authorization: ApiKey app.proxy1.broker.example.de AppKey" https://proxy1.broker.example.de/v1/tasks
```
`Proxy1` replies:
```
HTTP/1.1 201 Created
Location: /tasks/70c0aa90-bfcf-4312-a6af-42cbd57dc0b8
Content-Length: 0
Date: Mon, 27 Jun 2022 13:58:35 GMT
```
where the `Location` header field is the id of the newly created task. With that
the task is registered and will be distributed to the appropriate locations.

### Listening for relevant tasks
`app` at Party 2 is now able to fetch all tasks addressed to them, especially the task created before:
```
curl -k -X GET -v -H "Authorization: ApiKey app.proxy2.broker.example.de AppKey" https://proxy2.broker.example.de/v1/tasks?filter=todo
```
The `filter=todo` parameter instructs the Broker to only send unfinished tasks
addressed to the querying party.
The query returns the task, and as `app` at Proxy 2, we inform the broker that
we are working on this important task by creating a preliminary "result" with
`"status": "claimed"`:
```
curl -k -X PUT -v --json '{"from":"app.proxy2.broker.example.de","id":"8db76400-e2d9-4d9d-881f-f073336338c1","metadata":["Arbitrary","types","are","possible"],"status":"claimed","task":"70c0aa90-bfcf-4312-a6af-42cbd57dc0b8","to":["app.proxy1.broker.example.de"]}' -H "Authorization: ApiKey app.proxy2.broker.example.de AppKey" https://proxy2.broker.example.de/v1/tasks/70c0aa90-bfcf-4312-a6af-42cbd57dc0b8/results/app.proxy2.broker.example.de
```

### Returning a Result
Party 2 processes the received task. After succeeding, `app` at party 2 returns the result to party 1:
```
curl -k -X PUT -v --json '{"from":"app.proxy2.broker.example.de","metadata":["Arbitrary","types","are","possible"],"status":"succeeded","body":"The answer is 42","task":"70c0aa90-bfcf-4312-a6af-42cbd57dc0b8","to":["app.proxy1.broker.example.de"]}' -H "Authorization: ApiKey app.proxy2.broker.example.de AppKey" https://proxy2.broker.example.de/v1/tasks/70c0aa90-bfcf-4312-a6af-42cbd57dc0b8/results/app.proxy2.broker.example.de
```

### Waiting for tasks to complete
Meanwhile, `app` at party 1 waits on the completion of its task. But not wanting to check for results every couple seconds, it asks Proxy 1 to be informed if the expected number of `1` result is present:
```
curl -k -X GET -v -H "Authorization: ApiKey app.proxy1.broker.example.de AppKey" https://proxy1.broker.example.de/v1/tasks/70c0aa90-bfcf-4312-a6af-42cbd57dc0b8/results?wait_count=1
```
This *long polling* opens the connection and sleeps until a reply is received. For more information, see the API documentation.

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
  "body": "Much work to do",
  "failure_strategy": {
    "retry": {
      "backoff_millisecs": 1000,
      "max_tries": 5
    }
  },
  "ttl": 3600,
  "metadata": "The broker can read and use this field e.g., to apply filters on behalf of an app"
}
```

- `id`: UUID to identify the task. Note that when the task is initially submitted, the server is not required to use the submitted ID but may auto-generate its own one. Callers must assume the submission's `id` property is ignored and check the reply's `Location` header for the actual URL to the task.
- `from`: BeamID of the submitting applications. Is automatically set by the Proxy according to the authentication info.
- `to`: BeamIDs of *workers* allowed to retrieve the task and submit results.
- `body`: Description of work to be done. Not interpreted by the Broker.
- `failure_strategy`: Advises each client how to handle failures. Possible values `discard`, `retry`.
- `failure_strategy.retry`: How often to retry (`max_tries`) a failed task and how long to wait in between each try (`backoff_millisecs`).
- `ttl`: Time-to-live in seconds. Once this reaches zero, the broker will expunge the task along with its results.
- `metadata`: Associated data readable by the broker. Can be of arbitrary type (see [Result](#result) for more examples) and can be handled by the broker (thus intentionally not encrypted).

### Result
Each task can hold 0...n results by each *worker* defined in the task's `to` field.

A succeeded result for the above task:
```json
{
  "from": "app1.proxy-hd.broker-project1.samply.de",
  "to": [
    "app7.proxy-hd.broker-project1.samply.de"
  ],
  "task": "70c0aa90-bfcf-4312-a6af-42cbd57dc0b8",
  "status": "succeeded",
  "body": "Successfully quenched 1.43e14 flux pulse devices",
  "metadata": ["Arbitrary", "types", "are", "possible"]
}
```

A failed task:
```json
{
  "from": "app5.proxy-ma.broker-project1.samply.de",
  "to": [
    "app7.proxy-hd.broker-project1.samply.de"
  ],
  "task": "70c0aa90-bfcf-4312-a6af-42cbd57dc0b8",
  "status": "permfailed",
  "body": "Unable to decrypt quantum state",
  "metadata": {
    "complex": "A map (key 'complex') is possible, too"
  }
}
```

- `from`: BeamID identifying the client submitting this result. This needs to match an entry the `to` field in the task.
- `to`: BeamIDs the intended recipients of the result. Used for encrypted payloads.
- `task`: UUID identifying the task this result belongs to.
- `status`: Defines status of this work result. Allowed values `claimed`, `tempfailed`, `permfailed`, `succeeded`. It is up to the application how these statuses are used. For example, some application might require workers to acknowledge the receipt of tasks by setting `status=claimed`, whereas others have only short-running tasks and skip this step.
- `body`: Supported and required for all `status`es except for `claimed`. Either carries the actual result payload of the task in case the status is `succeeded` or an error message.
- `metadata`: Associated data readable by the broker. Can be of arbitrary type (see [Task](#task)) and is not encrypted.

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
Location: /tasks/b999cf15-3c31-408f-a3e6-a47502308799
Content-Length: 0
Date: Mon, 27 Jun 2022 13:58:35 GMT
```

In subsequent requests, use the URL defined in the `location` header to refer to the task (NOT the one you supplied in your POST body).

### Retrieve tasks
Workers regularly call this endpoint to retrieve submitted tasks.

Method: `GET`  
URL: `/v1/tasks`  
Parameters:
- `from` (optional): Fetch only tasks created by this ID.
- `to` (optional): Fetch only tasks directed to this ID.
- [long polling](#long-polling-api-access) is supported.
- `filter` (optional): Fetch only tasks fulfilling the specified filter criterion. Generic queries are not yet implemented, but the following "convenience filters" reflecting common use cases exist:
  - `filter=todo`: Matches unfinished tasks to be worked on by the asking client. Is a combination of:
    - `to` contains me and
    - `results` do not contain me.

Returns an array of tasks, cf. [here](#task)
```
HTTP/1.1 200 OK
Content-Type: application/json
Content-Length: 220
Date: Mon, 27 Jun 2022 14:05:59 GMT

[
  {
    "id": ...
  }
)
```

### Create a result
Create or update a result of a task. 

Method: `PUT`  
URL: `/v1/tasks/<task_id>/results/<app_id>`  
Body: see [Result](#result)  
Parameters: none

Returns:
```
HTTP/1.1 204 No Content
Content-Length: 0
Date: Mon, 27 Jun 2022 13:58:35 GMT
```

### Retrieve results
The submitter of the task (see [Create Task](#create-task)) calls this endpoint to retrieve the results.

Method: `GET`  
URL: `/v1/tasks/<task_id>/results`  
Parameters:
- [long polling](#long-polling-api-access) is supported.

Returns an array of results, cf. [here](#result)
```
HTTP/1.1 200 OK
Content-Type: application/json
Content-Length: 179
Date: Mon, 27 Jun 2022 14:26:45 GMT

[
  {
    "id": ...
  }
]
```

### Long-polling API access
As part of making this API performant, all reading endpoints support long-polling as an efficient alternative to regular (repeated) polling. Using this function requires the following parameters:
- `wait_count`: The API call will block until this many results are available ...
- `wait_time`: ... or this many milliseconds have passed, whichever comes first.

For example, retrieving a task's results:
- `GET /v1/tasks/<task_id>/results` will return immediately with however many results are available,
- `GET /v1/tasks/<task_id>/results?wait_count=5` will block forever until 5 results are available,
- `GET /v1/tasks/<task_id>/results?wait_count=5&wait_time=30000` will block until 5 results are available or 30 seconds have passed (whichever comes first). In the latter case, HTTP code `206 (Partial Content)` is returned to indicate that the result is incomplete.

### Health Check
To monitor the operational status of Samply.Beam, each component implements a specific health check endpoint.

Method: `GET`  
URL: `/v1/health`  
Parameters:
- None

In the current version only an appropriate status code is returned once/if initialization has succeeded. However, in the future more detailed health information might be returned in the reply body.
```
HTTP/1.1 200 OK
Content-Length: 0
Date: Mon, 27 Jun 2022 14:26:45 GMT
```

## Development Environment

A dev environment is provided consisting of one broker and two proxies. 

To start the dev setup:
```shell
./dev/beamdev start
```

Steps may fail and ask you to install tools. In particular, note that you need a current (>= 7.7.0) curl version.

Alternatively, you can run the services in the background and get the logs as follows:

```shell
./dev/beamdev start_bg
docker-compose logs -f
```

Confirm that your setup works by running `./dev/test noci`, which runs the tests against your instances.

To work with the environment, you may run `./dev/beamdev defaults` to see some helpful values, including the dev default URLs and a working authentication header.

### Logging
Both the Broker and the Proxy respect the log level in the `RUST_LOG` environment variable. E.g., `RUST_LOG=debug` enables debug outputs. Warning: the `trace` log level is *very* noisy.

## Roadmap
- [X] API Key authentication of local applications
- [X] Certificate management
- [X] End-to-End signatures
- [ ] End-to-End encryption
- [X] Docker deployment packages: CI/CD
- [ ] Docker deployment packages: Documentation
- [X] Broker-side filtering using pre-defined criteria
- [ ] Broker-side filtering of the unencrypted fields with JSON queries
- [ ] Integration of OAuth2 (in discussion)
- [x] Helpful dev environment
- [x] Expiration of tasks and results
- [x] Support TLS-terminating proxies

## Cryptography Notice
This distribution includes cryptographic software. The country in which you currently reside may have restrictions on the import, possession, use, and/or re-export to another country, of encryption software. BEFORE using any encryption software, please check your country's laws, regulations and policies concerning the import, possession, or use, and re-export of encryption software, to see if this is permitted. See [http://www.wassenaar.org/](http://www.wassenaar.org) for more information.
