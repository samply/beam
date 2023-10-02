# Samply.Beam 0.7.0 – 2023-10-04

This major release features almost 300 commits introducing multiple improvements, new features, and bug fixes. In particular, we are thrilled to introduce the possibility to use Samply.Beam for secure and easy *direct socket connections*. This opens Samply.Beam for many additional use cases, where a message-passing approach is not suitable, e.g. applications with high bandwidth demands.

## Breaking changes
* *Streamlined Configuration:* To reduce configuration redundancy, the environment variable schema of the Beam.Proxies for AppIDs and related App-API Keys has changed. Both parameters are now consolidated in the form: `APP_<AppId>_KEY=<API-Key>`.
* *Task Creation Bugfix:* A previous bug, where messages where the recipient list includes recipients with invalid/expired certificates was not handled properly, is now fixed. We decided to let the task creation in those cases fail to notify the client application of the issue by sending a status code `424 Failed Dependency`, instead of dropping the offending recipient quietly. 
* *Time Units for Parameters:* Both, a Task's `ttl` field and the `wait_time` long polling parameter now expects not only an integer, but a specified time unit as well, e.g., `24h`, `5m`, `25ms`, etc.

## Major changes
* *Direct Socket Connection:* Samply.Beam can now securely transport direct socket connections. For many applications the message/task-based communication model is not a great fit, e.g. if many, many very small packages must be transmitted or if network bandwidth utilization is paramount. For those cases, Samply.Beam can now establish socket connections. As this feature might not be compatible with every data protection concept, it is only enabled if the corresponding `sockets` feature flag is set during compilation. Similarly, in addition to the usual `main` and `develop` docker image tags, the tags `main-sockets` and `develop-sockets` are published as well.
* *Improved Certificate Caching and Management:* The certificate caching and management in the Beam.Proxy components have been vastly improved. This greatly reduces the communication overhead between proxy and broker. Note: If the proxy's own certificate changes, the proxy exits to restart with the new certificate.
* *Health monitoring:* In parallel to the task/result transmitting connection between proxies and broker, a *permanent control connection* is established. Currently, this connection is only used to determine the online status of the proxies and monitor for unexpected connection interuptions, however, in the future additional signaling can utilize this channel.
* *Introducing `beam-lib` Crate:* The project now contains a separate crate, `beam-lib`, exposing many data structures such as BeamIds and Task/Result structs. This crate can be used by application developers to easily interface with a beam proxy in the Rust programming language.
* *Enhanced Broker Health Monitoring:* The Broker `health` Endpoint is much more verbose and can -- thanks to the control connection -- return information regarding the proxy connection status. This makes monitoring in a highly federated system much simpler.
* *Certificate Management:* Support for revocation of Beam certificates. Enhanced cert management companion tools.

## Minor changes
* *Revamped Internal Event Handling:* The internal message queue of the broker has been completely refactored. The broker now employs a much more elegant event manager to handle tasks, expirations, and other events. Not only does this improve the efficiency, the new system is better maintainable and easily extensible in the future.
* *Improved Wire Format:* The serialization of the Tasks and Results have been improved. The tasks should now require around a third less communication.
* *Refined Logging and Error Handling:* Many improvements in logging, error handling, and the expressiveness of the return values have been implemented.
* *Enhanced Beam.Broker Efficiency:* By using more efficient concurrent data structures, the efficiency of the Beam.Broker operation has been improved.
* *Streamlined CI/CD Pipeline:* The CI/CD Pipeline has been tweaked to allow faster compile and testing cycles.
* *Verbose User-Agent for Development Builds:* For non-`main` builds of Samply.Beam, the User-Agent identifies the git commit hash of the component for debugging purposes.
* *Expanded Testing:* Additional tests have been added and refactoring efforts to include all integration tests fully into the Rust testing framework have been started.
* *Dependency Maintenance:* All used dependencies of Samply.Beam have been pruned and updated.
* *Made Debug MITM Proxy optional:* The MITM proxy for debugging, introduced in version 0.6.0 has been commented out in dev/docker-compose.yml, as it interferes with SSE, and hence, beamdev demo.

## Bugfixes
* In addition to the "recipient with invalid certificate" bug described under [Breaking changes](#breaking-changes), many small bugs dealing with certificate retrieval and concurrency have been fixed in this release.
* Under some circumstances, the Beam.Broker would deadlock when using the SSE interface. This is now fixed.

# Samply.Beam 0.6.1 -- 2023-04-11

This minor easter update is just a maintenance release. We updated our time-parsing dependency [fundu](https://crates.io/crates/fundu) to the next major version and fixed a bug in our CI/CD pipeline. With this fix, the project description is now correctly sent to Docker Hub. Internally, we improved the formatting of the source code.

# Samply.Beam 0.6.0 -- 2023-03-30

Samply.Beam version 0.6.0 represents another major milestone in the Beam development roadmap. We improved our network communication sizes by more than a factor of two, added (experimental) Server-Sent-Events for more efficient communication and heavily refactored the codebase to provide better maintainability and robustness. For all times, i.e. `wait_time` calls and BeamTask's `ttl` (time-to-live) field, the unit of time can be specified. This comes at the cost of an external API change: please adapt your applications to send the `ttl` as a string. Of course, this release, again, contains a lot of quality-of-life improvements regarding logging, building, development setups, etc. The following changelog gives more details.

## Breaking changes

* Improvement of internal message efficiency:
In previous releases, the encrypted payload and the encapsulated encryption keys, both fields byte arrays, were encoded as JSON arrays of the ASCII representation of the corresponding decimal numbers. This, of course, potentially quadruples the payload size. We chose a base64-string encoding for those fields to strike a balance against network efficiency and encoding performance. Other encoding types, such as base85, turned out to be (depending on the payload) around 1300% slower.
* All times given to Beam, both the time-to-live (ttl) field in the Beam Task and the `wait_time` long-polling parameter can be used with time units by adding `h` for hours, `m`for minutes, `ms` for milliseconds and so on. If no unit is given, seconds (`s`) is assumed. As this changes the `ttl` field from an integer to a string (with mandatory quotation marks), this is a braking change.
* The log level for the hyper component (HTTP handling) is now set to `warn`, except if explicitly specified otherwise, e.g., by setting `RUST_LOG="debug,hyper=debug"`.

## Major changes

* We improved the resilience of the communication between the Beam.Broker and the central PKI (Hashicorp Vault) by a more explicit retry mechanism and improved logging.
* We updated the certificate cache, resulting in a) less network communication and b) less parsing, hence improving performance and eliminating potential sources of errors.
* The `v1/health` endpoint of Samply.Broker gives more information regarding the current system status, e.g. if the central vault is reachable/sealed.
* A first experimental implementation of Server-sent Events (SSE) for fetching results is implemented. To use this API, set the request header `Accept: text/event-stream`. Note, however, that this feature is still experimental and subject to changes.

## Minor improvements

* We improved the dev build script to avoid out-of-sync binary and docker image generation.
* The logging was improved throughout the board. Some Information were reduced in severity to `trace` level.
* Beam development is now supported on both libssl1.1 and libssl3 Linuxes (e.g. Ubuntu 20.04 vs. Ubuntu 22.04). With the impending EOL of libssl 1.1, we hope for quick transition of the main linux distribution providers to fully remove libssl1.1 support in a future release.
* Beam development will now automatically determine when to rebuild the Docker images.
* Beam now gracefully (and quickly) exits in Docker environments where not all Unix signals are forwarded into containers.
* `beamdev start` now starts a MITM proxy for debugging (access at http://localhost:9090)
* Beams message types were heavily refactored for improved maintainability and cleaner, more ideomtaic code. 

## Bugfixes
* A bug, where some messages' signatures from the Beam.Broker to the Beam.Proxy were not properly validated, is fixed.
* We fixed a bug, where the logging engine might be initialized and and lost some startup messages.

# Samply.Beam 0.5.0 -- 2023-02-03

This new major release of Samply.Beam introduces a breaking change in how cryptographic signatures are represented in the HTTP messages, allowing for more efficient, larger payloads.
This change is incompatible with older versions of Samply.Beam, so please update both your broker and your clients.

# Samply.Beam 0.4.2 -- 2023-02-01

In this first minor release of the year, the following minor improvements were implemented:

* The default log level is now "info", instead of "error"
* The error handling, especially regarding proxy certificate validation, has been improved
* Additional internal retry amd timeout mechanisms, e.g. for the communication between Proxy and Broker, and the Broker and Vault, are in place.
* Beam.Broker and Beam.Proxy now set the appropriate header fields, such as VIA and SERVER.
* Improvements to internal code quality and documentation

# Samply.Beam 0.4.1 -- 2022-12-08

This is a bugfix release. In particular, the following things were addressed:

* An error, where only the first result from multiple was decrypted is fixed.
* Due to computer time mismatches, sometimes JWTs "from the future" were rejected. The Broker now accepts those tokens. Note, that expired tokens are still rejected.

# Samply.Beam 0.4.0 -- 2022-12-07

This newest release of Samply.Beam paves the road to more production safety. We revised the CA deployment and usage, established a clearly defined (and tool supported) certificate enrollment process, and improved the overall stability of the applications. Additionally, the end-to-end encryption is fully operational in this version.

The new certificate and private key handling process requires manual intervention in the update process, so please carefully read the following change notice.

We are exited to release this new version and help with shaping the infrastructure landscape of the European medical informatics.

## Breaking changes

### Certificate enrollment

In previous releases of Beam, certificate and private key generation was entirely left to the system administrator. Furthermore, only the certificate expiry but not the certificate chain validated. Now, both the Beam.Proxy and the Beam.Broker require the CA's root certificate to be present locally and validates the Root CA --> Intermediate CA --> Proxy Certificate chain. For that, the command line parameter `--rootcert-file`, and the environment variable `ROOTCERT_FILE` have been introduced. The [Beam deployment packages]() reflect this change by loading the root certificate via the docker-compose secret mechanism.

Furthermore, the process of creating a private key and enrolling a matching certificate into the CA has been described in the documentation and a companion tool to assist system administrators [has been published](). The companion tool takes the proxy id and broker id and a) generates a private key in a location of choice and b) prints out a Certificate Sign Request (CSR), that can be signed at the central CA. This way, we hope to ensure, that the private key never leaves the local sites.

You can find more information in the [documentation](README.MD#beam-enrollment-companion-tool-assisted-method) and in the repository for [Beam's central docker-compose deployment]().

### Certificate validation and selection

Previously, if multiple certificates for a single common name were present in the CA the first one retrieved was chosen by the Beam.Proxy to use. In this version, the logic of choosing a certificate has changed: First, expired certificates are discarded. Second, certificates using a different modulus than the private key are discarded, and lastly, the newest of the remaining certificates are chosen.

Future versions of Beam will include an additional Certificate Revocation List check, however, this is not implemented, yet.

If your current setup relies on the previous behaviour, please check your CA and setup the certificates to follow the previously outlined selection rules.

### End-to-end encryption

This version fully implements end-to-end-encryption between the Beam.Proxies. This breaks downwards compatibility and requires all Beam.Proxies, as well as the central Beam.Broker, to run at least this version (v0.4.0).

## Major changes

* Bugfix: `Wait_count` malfunction. There was an issue, where some results were not returned by the long polling mechanism. The issue is fixed in this version.

## Minor improvement

* The `todo` filter does fetch `claimed` and `tempfailed` tasks, to allow resuming them
* Improved debug output and logging
* Updated dependencies
* Removed `vault-rs` and `tower` dependencies

# Samply.Beam 0.3.0 -- 2022-09-26

We are happy to announce the new Samply.Beam major release 0.3.0. Like many early-stage software projects, the Samply.Beam developments moves fast and we are exited to see new features, improved performance and stability, and an overall improvement of code quality and error handling. Furthermore, the first productive use of Samply.Beam brought some shortcomings in the API to our attention, so we strived to streamline the developer experience for costumer applications.

Unfortunately, this high pace of development and the significant improvements come with some breaking changes in the API. If you have not already, please adapt your applications accordingly.

There are still some interesting features planned and scheduled, some of which will introduce another breaking API change. However, we expect to reach a more stable state in a few months.

Thank you for using Samply.Beam and all your feedback, helping us improve Samply.Beam further.

## Breaking changes

### Expiry of messages

Many tasks are only relevant for a specific duration. This is now reflected in the mandatory `ttl` field of the task JSON, stating the time-to-live in seconds. After this time, the task and all its attached results expire and are cleaned up.

### Removed `status` body for `Result` objects

To be consistent in the usage of the `Task` and `Result` JSONs, we changed the
`Result` interface. The `status` now only provides the status as a string (i.e.,
`claimed`, `succeeded`, `tempfailed`, or `permfailed`) and does not take the
return body itself. Instead, the `body` of a successful result is attached to
the root of the `Result` object.

### Cleaned up public API

* Removed `id` field from result
* Changed result endpoint

## Major changes

### The Beam.Proxies now address the central CA via Beam.Broker

By addressing the central CA via central Broker the configuration of the local Beam.Proxies is greatly reduced and the central CA does not need to be publicly reachable.

### Support for TLS terminating proxy servers

As many institutions operate a Man-in-the-middle TLS-terminating proxy, you can now provide the appropriate certificate to successfully establish connections through theses systems. The optional command line parameter `--tls_ca_certificates_dir` (or the respective environment variable) takes the folder containing all relevant certificates.

### Respect `no_proxy` environment variable

Due to the proxy traversal system overhaul, Beam.Broker and Beam.Proxy now correctly uses the `no_proxy` environment variable. As before, `http_proxy` and `https_proxy`, or as a new option `all_proxy`, are used for proxy usage.

### Cross compilation CI/CD pipeline for arm64 images

A new, higher performance CI/CD pipeline automatically builds Beam docker images not only for amd64, but for arm64 architectures as well.

## Minor improvements

* Show an informational banner at startup
* Respect the `RUST_LOG` variable for log level
* Improved testing
* Internal improvements
