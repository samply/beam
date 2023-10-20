![Logo](./doc/Logo.svg) <!-- TODO: New Logo -->

[![Build with rust and docker](https://github.com/samply/beam/actions/workflows/rust.yml/badge.svg)](https://github.com/samply/beam/actions/workflows/rust.yml)

Samply.Beam is a distributed task broker designed for efficient communication across strict network environments. It provides most commonly used communication patterns across strict network boundaries, end-to-end encryption and signatures, as well as certificate management and validation on top of an easy to use REST API. In addition to task/response semantics, Samply.Beam supports high-performance applications with encrypted low-level direct socket connections.

## Latest version: Samply.Beam 0.7.0 – 2023-10-04

This new version introduces many new features, such as direct socket connections, improved certificate caching and health monitoring for the Proxy (via a long-lived control connection) and the Broker. As indicated by the [major version change](https://semver.org/), some breaking changes have been introduced. Please check the [Changelog](CHANGELOG.md) for details.

Find info on all previous versions in the [Changelog](CHANGELOG.md).

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
- Finally, the *AppId* (`app3...`) results from using the correct API key in communication with the Proxy (Header `Authorization: ApiKey app3.broker2.samply.de <app3's API key>`)

In practice,

- there is one Broker per research network (`broker1.samply.de`)
- each site has one Bridgehead with one Proxy instance (`proxy2` for site #2)
- many apps use `proxy2` to communicate within the network (`app1`, `app2`, `app3`, ...)

This design ensures that each component, mainly applications but Proxies and Brokers as well, can be addressed in tasks. Should the need arise in the future, this network could be federated by federating the brokers (not unsimilar to E-Mail/SMTP, XMPP, etc.)

The Proxies have to fetch certificates from the central Certificate Authority, however, this communication is relayed by the broker. This ensures, that no external access to the CA is required.

## Getting started

Using Docker, you can run a small demo beam network by checking out the git repository (use `main` or `develop` branch) and running the following command:
```bash
./dev/beamdev demo
```
This will launch your own beam demo network, which consists of one broker (listening on `localhost:8080`) and two connected proxies (listening on `localhost:8081` and `localhost:8082`).

The following paragraph simulates the creation and the completion of a task
using [cURL](http://curl.se) calls. Two parties (and their Samply.Proxies) are
connected via a central broker. Each party has one registered application.
In the next section we will simulate the communication between these applications over the beam network.

> Note: cURL versions before 7.82 do not support the `--json` option. In this case, please use `--data` instead.

The used BeamIds are the following:

| System             | BeamID                       |
|--------------------|------------------------------|
| Broker             | broker                       |
| Proxy1             | proxy1.broker                |
| App behind Proxy 1 | app1.proxy1.broker           |
| Proxy2             | proxy2.broker                |
| App behind Proxy 2 | app2.proxy2.broker           |

To simplify this example, we use the same ApiKey `App1Secret` for both apps. Also, the Broker has a short name (`broker`) where in a real setup, it would be required to have a fully-qualified domain name as `broker1.samply.de` (see [System Architecture](#system-architecture)).

### Creating a task

`app1` at party 1 has some important work to distribute. It knows, that `app2`
at party 2 is capable of solving it, so it asks `proxy1.broker` to
create that new task:

```sh
curl -v --json '{"body":"What is the answer to the ultimate question of life, the universe, and everything?","failure_strategy":{"retry":{"backoff_millisecs":1000,"max_tries":5}},"from":"app1.proxy1.broker","id":"70c0aa90-bfcf-4312-a6af-42cbd57dc0b8","metadata":"The broker can read and use this field e.g., to apply filters on behalf of an app","to":["app2.proxy2.broker"],"ttl":"60s"}' -H "Authorization: ApiKey app1.proxy1.broker App1Secret" http://localhost:8081/v1/tasks
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

`app2` at Party 2 is now able to fetch all tasks addressed to them, especially the task created before:

```sh
curl -X GET -v -H "Authorization: ApiKey app2.proxy2.broker App1Secret" http://localhost:8082/v1/tasks?filter=todo
```

The `filter=todo` parameter instructs the Broker to only send unfinished tasks
addressed to the querying party.
The query returns the task, and as `app2` at Proxy 2, we inform the broker that
we are working on this important task by creating a preliminary "result" with
`"status": "claimed"`:

```sh
curl -X PUT -v --json '{"from":"app2.proxy2.broker","id":"8db76400-e2d9-4d9d-881f-f073336338c1","metadata":["Arbitrary","types","are","possible"],"status":"claimed","task":"70c0aa90-bfcf-4312-a6af-42cbd57dc0b8","to":["app1.proxy1.broker"]}' -H "Authorization: ApiKey app2.proxy2.broker App1Secret" http://localhost:8082/v1/tasks/70c0aa90-bfcf-4312-a6af-42cbd57dc0b8/results/app2.proxy2.broker
```

### Returning a Result

Party 2 processes the received task. After succeeding, `app2` returns the result to party 1:

```sh
curl -X PUT -v --json '{"from":"app2.proxy2.broker","metadata":["Arbitrary","types","are","possible"],"status":"succeeded","body":"The answer is 42","task":"70c0aa90-bfcf-4312-a6af-42cbd57dc0b8","to":["app1.proxy1.broker"]}' -H "Authorization: ApiKey app2.proxy2.broker App1Secret" http://localhost:8082/v1/tasks/70c0aa90-bfcf-4312-a6af-42cbd57dc0b8/results/app2.proxy2.broker
```

### Waiting for tasks to complete

Meanwhile, `app1` waits on the completion of its task. But not wanting to check for results every couple seconds, it asks Proxy 1 to be informed if the expected number of `1` result is present:

```sh
curl -X GET -v -H "Authorization: ApiKey app1.proxy1.broker App1Secret" http://localhost:8081/v1/tasks/70c0aa90-bfcf-4312-a6af-42cbd57dc0b8/results?wait_count=1
```

This *long polling* opens the connection and sleeps until a reply is received. For more information, see the API documentation.

### Using direct socket connections
> Only available on builds of beam with the `sockets` feature 

Establishing direct socket connections via Beam requires a negotiation phase prior to using the sockets. One application sends a socket request to the other application via their respective Beam.Proxy. The receiving application, upon receipt of the request, upgrades the connection to an encrypted TCP socket connection.

While Beam sockets can be initiated using command line tools, such as curl, netcat, or socat, they are intended to be used by the applications' code. Thus, we show the usage in the following short python application exemplifying both applications, the initiating and the receiving one. Both sides of the communication run concurrently.

```python
import requests
import threading
import socket

data = b"Hello beam sockets!"

# App running in a beam network with proxy1 available at localhost:8081
def app1():
    # Post socket request to client
    res = requests.post("http://localhost:8081/v1/sockets/app2.proxy2.broker", headers = {
        "Upgrade": "tcp",
        "Authorization": "ApiKey app1.proxy1.broker App1Secret"
    }, stream=True)
    # Get the underlying socket connection
    stream = socket.fromfd(res.raw.fileno(), socket.AF_INET, socket.SOCK_STREAM)
    # Send some data
    stream.send(data)

# App running in a beam network with proxy2 available at localhost:8082
def app2():
    # Poll for incoming socket requests
    socket_task_id = requests.get("http://localhost:8082/v1/sockets", headers={
        "Authorization": "ApiKey app2.proxy2.broker App1Secret"
    }).json()[0]["id"]
    # Connect to the given id of the socket request
    res = requests.get(f"http://localhost:8082/v1/sockets/{socket_task_id}", headers={
        "Authorization": "ApiKey app2.proxy2.broker App1Secret",
        "Upgrade": "tcp",
    }, stream=True)
    # Get the underlying socket connection
    stream = socket.fromfd(res.raw.fileno(), socket.AF_INET, socket.SOCK_STREAM)
    # Receive the data send by the other client
    assert stream.recv(len(data)) == data

threading.Thread(target=app1).start()
threading.Thread(target=app2).start()
```


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
  "ttl": "30s",
  "metadata": "The broker can read and use this field e.g., to apply filters on behalf of an app"
}
```

- `id`: UUID to identify the task. Note that when the task is initially submitted, the server is not required to use the submitted ID but may auto-generate its own one. Callers must assume the submission's `id` property is ignored and check the reply's `Location` header for the actual URL to the task.
- `from`: BeamID of the submitting applications. Is automatically set by the Proxy according to the authentication info.
- `to`: BeamIDs of *workers* allowed to retrieve the task and submit results.
- `body`: Description of work to be done. Not interpreted by the Broker.
- `failure_strategy`: Advises each client how to handle failures. Possible values `discard`, `retry`.
- `failure_strategy.retry`: How often to retry (`max_tries`) a failed task and how long to wait in between each try (`backoff_millisecs`).
- `ttl`: Time-to-live. If not stated differently (by adding 'm', 'h', 'ms', etc.), this value is interpreted as seconds. Once this reaches zero, the broker will expunge the task along with its results.
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

### Socket Task
> Only available on builds of beam with the `sockets` feature 

While "regular" Beam Tasks transport application data, Socket Task initiate direct socket connections between two Beam.Proxies.

```json
{
    "from": "app1.proxy1.broker",
    "to": ["app2.proxy2.broker"],
    "id": "<socket_uuid>",
    "ttl": "60s",
    "metadata": "some custom json value"
}
```

- `from`: BeamID of the client requesting the socket connection
- `to`: BeamIDs of the intended recipients. Due to the nature of socket connections, the array has to be of exact length 1.
- `id`: A UUID v4 which identifies the socket connection and is used by the recipient to connect to this socket (see [here](#connecting-to-a-socket-request)).
- `ttl`: The time-to-live of this socket task. After this time has elapsed the recipient can no longer connect to the socket. Already established connections are not affected.
- `metadata`: Associated unencrypted data. Can be of arbitrary type same as in [Task](#task).
## API

### Create task

Create a new task to be worked on by defined workers. Currently, the body is restricted to 10MB in size.

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

If the task contains recipients (`to` field, see [Beam Task](#task)) with invalid certificates (i.e. not certificate exists or it expired), Beam *does not* create the task but returns HTTP status code `424 Failed Dependency` with a JSON array of the "offending" BeamIDs in the body, e.g.:

```
HTTP/1.1 424 Failed Dependency
Content-Length: 34
Date: Thu, 28 Sep 2023 07:16:24 GMT

["proxy4.broker", "proxy6.broker"]
```

In this case, remove or correct these BeamIDs from the `to` field of your task and re-send.

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
    - `results` do not contain a result from me (except results with `status` values of `claimed,tempfail`, to allow resuming those tasks).

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

Create or update a result of a task. Currently, the body is restricted to 10MB in size.

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
- `wait_time`: ... or this time has passed (if not stated differently, e.g., by adding 'm', 'h', 'ms', ..., this is interpreted as seconds), whichever comes first.

For example, retrieving a task's results:

- `GET /v1/tasks/<task_id>/results` will return immediately with however many results are available,
- `GET /v1/tasks/<task_id>/results?wait_count=5` will block forever until 5 results are available,
- `GET /v1/tasks/<task_id>/results?wait_count=5&wait_time=30s` will block until 5 results are available or 30 seconds have passed (whichever comes first). In the latter case, HTTP code `206 (Partial Content)` is returned to indicate that the result is incomplete.

### Server-sent Events (SSE) API (experimental)

To better support asynchronous use cases, such as web-based user interfaces streaming results, this development version supports a first implementation of [Server-Sent Events](https://www.rfc-editor.org/rfc/rfc8895.html#name-server-push-server-sent-eve) for *Result* retrieval. This allows Beam.Proxies to "subscribe" to tasks and get notifications for every new result without explicit polling. Similar to WebSockets, this is supported natively by JavaScript in web browsers. However, in contrast to WebSockets, SSE are standard long-lived HTTP requests that is likely to pass even strict firewalls.

Please note: This feature is experimental and subject to changes.

Method: `GET`  
URL: `/v1/tasks/<task_id>/results?wait_count=3`  
Header: `Accept: text/event-stream`  
Parameters:

- The same parameters as for long-polling, i.e. `to`, `from`, `filter=todo`, `wait_count`, and `wait_time` are supported.

Returns a *stream* of results, cf. [here](#result):

```
HTTP/1.1 200 OK
Content-Type: text/event-stream
Cache-Control: no-cache
Transfer-Encoding: chunked
Date: Thu, 09 Mar 2023 16:28:47 GMT

event: new_result
data: {"body":"Unable to decrypt quantum state","from":"app2.proxy1.broker","metadata":{"complex":"A map (key complex) is possible, too"},"status":"permfailed","task":"70c0aa90-bfcf-4312-a6af-42cbd57dc0b8","to":["app1.proxy1.broker"]}

event: new_result
data: {"body":"Successfully quenched 1.43e14 flux pulse devices","from":"app1.proxy1.broker","metadata":["Arbitrary","types","are","possible"],"status":"succeeded","task":"70c0aa90-bfcf-4312-a6af-42cbd57dc0b8","to":["app1.proxy1.broker"]}

[...]
```

You can consume this output natively within many settings, including web browsers. For more information, see [Mozilla's developer documentation](https://developer.mozilla.org/en-US/docs/Web/API/Server-sent_events/Using_server-sent_events)

### Health Check

To monitor the operational status of Samply.Beam, each component implements a specific health check endpoint.

Method: `GET`  
URL: `/v1/health`  
Parameters:

- None

In the current version, the Beam.Proxy only returns an appropriate status code once/if initialization has succeeded. However, in the future more detailed health information might be returned in the reply body.

```
HTTP/1.1 200 OK
Content-Length: 0
Date: Mon, 27 Jun 2022 14:26:45 GMT
```

The Beam.Broker implements a more informative health endpoint and returns a health summary and additional system details:

```
HTTP/1.1 200
{
  "summary": "healthy",
  "vault": {
    "status": "ok"
  }
}
```

or in  case of an issue, e.g.:

```
HTTP/1.1 503
{
  "summary": "unhealthy",
  "vault": {
    "status": "unavailable"
  }
}
```

Additionally, the broker health endpoint publishes the connection status of the proxies:

Method: `GET`  
URL: `/v1/health/proxies/<proxy-id>`  
Authorization:

 - Basic Auth with an empty user and the configured `MONITORING_API_KEY` as a password, so the header looks like `Authorization: Basic <base64 of ':<MONITORING_API_KEY>'>`.

In case of a successful connection between proxy and broker, the call returns HTTP status code `200 OK`, otherwise `404 Not Found`.

Querying the endpoint without specifying a ProxyId returns a JSON array of all proxies, that have ever connected to this broker:

Method: `GET`  
URL: `/v1/health/proxies`  
Authorization:

 - Basic Auth with an empty user and the configured `MONITORING_API_KEY` as a password, so the header looks like `Authorization: Basic <base64 of ':<MONITORING_API_KEY>'>`.

yields, for example,

```
HTTP/1.1 200
[
  "proxy1.broker.example",
  "proxy2.broker.example",
  "proxy3.broker.example",
  "proxy4.broker.example"
]
```

### Socket connections
> Note: Only available on builds with the feature `sockets` enabled. Both proxy and broker need to be built with this flag. There are also prebuilt docker images available with this feature.

All API requests require the usual authentication header (see [getting started section](#getting-started)).

#### Initialize a socket connection
Initialize a socket connection with an Beam application, e.g. with AppId `app2.proxy2.broker`:

Method: `POST`  
URL: `/v1/sockets/<app_id>`  
Header `Upgrade` is required, e.g. 'Upgrade: tls'
Optionally takes a `metadata` header which is expected to be serialized json value.
This corresponds to the `metadata` field on [Socket task](#socket-task).

This request will automatically lead to a connection to the other app, after it answers this request.

#### Receive and answer a socket request
To receive socket connections, the Beam.Proxy needs to be polled for incoming connections.
This endpoint also supports the [long polling](#long-polling-api-access) query string semantics.

Method: `GET`  
URL: `/v1/sockets`
Parameters:
 * The same parameters as for long-polling, i.e. `to`, `from`, `filter=todo`, `wait_count` and `wait_time` are supported.

Returns an array of JSON objects:
``` json
[
    {
        "from": "app1.proxy1.broker",
        "to": ["app2.proxy2.broker"],
        "id": "<socket_uuid>",
        "ttl": "60s",
        "metadata": "Some json value"
    }
]
```

#### Connecting to a socket request
After the connection negotiation above, the App can proceed to connect to the socket:

Method: GET  
URL: `/v1/sockets/<socket_uuid>`


## Development Environment

A dev environment is provided consisting of one broker and two proxies as well as an optional MITM proxy (listening on `localhost:9090`) for debugging. To use it, remove the comment signs for the MITM service and the `ALL_PROXY` environment variables in `dev/docker-compose.yml`. Note that the MITM proxy interferes with SSE. 

> NOTE: The commands in this section will build the beam proxy and broker locally. To build beam, you need to install libssl-dev.

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

To run the dev setup with additional cargo flags like feature flags or the release flag you may run `dev/beamdev start <cargo flags>`, i.e. `dev/beamdev start --features sockets`.

## Production Environment & Certificate Infrastructure

A production system needs to operate a production-hardened central [Hashicorp Vault](https://www.vaultproject.io/) and requires a slightly more involved secret management process to ensure, that no secret is accidentally leaked. We can give no support regarding the vault setup, please see the [official documentation](https://developer.hashicorp.com/vault/docs/secrets/pki). However, our [deployment repositories](https://github.com/samply/beam-deployment) have a basic vault cookbook section, describing a basic setup and the most common operations.

While the development system generates all secrets and certificates locally at startup time, the production system should a) persist the Beam.Proxy certificates at the central CA, and b) allow an easy private key generation and certificate enrollment. As the central components and the Beam.Proxies could be operated by different institutions, (private) key generation must be performed at the sites without involvement of the central CA operators.

Beam.Broker and Beam.Proxy expect the private key as well as the CA root certificate to be present at startup (the location can be changed via the `--rootcert-file` and `--privkey-file` command line parameters, as well as the corresponding environment variables). Furthermore, the certificates for the Beam.Proxy common names corresponding to those private keys must be available in the central CA. That means that the Proxy sites must generate a) a private key, b) a certificate request for signing before operation can commence. There are two possible ways to do that:

### Method 1: Using the Beam Enrollment Companion Tool

We created a [certificate enrollment companion tool](https://github.com/samply/beam-enroll), assisting the enrollment process. Please run the docker image via:

```bash
docker run --rm -v <output-directory>:/pki samply/beam-enroll:latest --output-file /pki/<proxy_name>.priv.pem --proxy-id <full_proxy_id>
```

and follow the instructions on the screen. The tool generates the private key file in the given directory and prints the CSR to the console -- ready to be copied into an email to the central CA's administrator without the risk of accidentally sending the wrong, i.e. private, file.

### Method 2: Using OpenSSL (manual)

The manual method requires `openssl` to be installed on the system.

To generate both required files simultaneously using the openssl command line tool, enter:

```bash
openssl req -nodes -new -newkey rsa:4096 -sha256 -out <proxy_name>.csr.pem
```

This generates both the private key and the CSR with the given name. Please note, that the private key must remain confidential and at your site!

Next, send the CSR to the central CA's administrator for signing and enrolling the proxy certificate.

### Logging

Both the Broker and the Proxy respect the log level in the `RUST_LOG` environment variable. E.g., `RUST_LOG=debug` enables debug outputs. Warning: the `trace` log level is *very* noisy.

## Technical Background Information

### End-to-End Encryption

Samply.Beam encrypts all information in the `body` fields of both Tasks and Results. The data is encryted in the Samply.Proxy before forwarding to the Beam.Broker. Similarly, the decryption takes place in the Beam.Proxy as well. This is in addition to the transport encryption (TLS) and different in that even the broker is unable to decipher the message's content fields.

The data is symmetrically encrypted using the Authenticated Encryption with Authenticated Data (AEAD) algorithm "XChaCha20Poly1305", a widespread algorithm (e.g., mandatory for the TLS protocol), regarded as highly secure by experts. The used [chacha20poly1305 library](https://docs.rs/chacha20poly1305/latest/chacha20poly1305/) was sublected to a [security audit](https://research.nccgroup.com/2020/02/26/public-report-rustcrypto-aes-gcm-and-chacha20poly1305-implementation-review/), with no significant findings. The randomly generated symmetric keys are encapsulated in a RSA encrypted ciphertext using OAEP Padding. This ensures, that only the intended recipients can decrypt the key and subsequently the transferred data.

## Roadmap

- [X] API Key authentication of local applications
- [X] Certificate management
- [X] Certificate enrollment process
- [X] End-to-End signatures
- [X] End-to-End encryption
- [X] Docker deployment packages: CI/CD
- [X] Broker-side filtering using pre-defined criteria
- [x] Helpful dev environment
- [x] Expiration of tasks and results
- [x] Support TLS-terminating proxies
- [x] Transport direct socket connections
- [x] Crate to support the development of Rust Beam client applications
- [ ] File transfers (with efficient support for large files)
- [ ] Broker-side filtering of tasks using the unencrypted metadata fields (probably using JSON queries)
- [ ] Integration of OAuth2 (in discussion)
- [ ] Deliver usage metrics

## Cryptography Notice

This distribution includes cryptographic software. The country in which you currently reside may have restrictions on the import, possession, use, and/or re-export to another country, of encryption software. BEFORE using any encryption software, please check your country's laws, regulations and policies concerning the import, possession, or use, and re-export of encryption software, to see if this is permitted. See [http://www.wassenaar.org/](http://www.wassenaar.org) for more information.
