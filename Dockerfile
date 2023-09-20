# This assumes binaries are present, see COPY directive.

FROM alpine AS chmodder
ARG FEATURE
ARG TARGETARCH
ARG COMPONENT
COPY /artifacts/binaries-$TARGETARCH$FEATURE/$COMPONENT /app/component
RUN chmod +x /app/*

FROM gcr.io/distroless/cc-debian12
COPY --from=chmodder /app/component /usr/local/bin/
ENTRYPOINT [ "/usr/local/bin/component" ]
