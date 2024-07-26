# This Dockerfile is infused with magic to speedup the build.
# In particular, it requires built binaries to be present (see COPY directive).
#
# tl;dr: To make this build work, run
#   ./dev/beamdev build
# and find your freshly built images tagged with the `localbuild` tag.

FROM alpine AS chmodder
ARG FEATURE
ARG TARGETARCH
ARG COMPONENT
COPY /artifacts/binaries-$TARGETARCH$FEATURE/$COMPONENT /app/$COMPONENT
RUN chmod +x /app/*

FROM gcr.io/distroless/cc-debian12
ARG COMPONENT
COPY --from=chmodder /app/$COMPONENT /usr/local/bin/samply
ENTRYPOINT [ "/usr/local/bin/samply" ]
