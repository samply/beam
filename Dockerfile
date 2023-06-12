# This assumes binaries are present, see COPY directive.

ARG IMGNAME=gcr.io/distroless/cc

FROM alpine AS chmodder
ARG FEATURE
ARG TARGETARCH
ARG COMPONENT
COPY /artifacts/binaries-$TARGETARCH$FEATURE/$COMPONENT /app/
RUN chmod +x /app/*

FROM ${IMGNAME}
COPY --from=chmodder /app/* /usr/local/bin/
ENTRYPOINT [ "/usr/local/bin/$COMPONENT" ]
