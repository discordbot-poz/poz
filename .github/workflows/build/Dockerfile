# syntax=docker/dockerfile:1
FROM --platform=$BUILDPLATFORM busybox:latest AS env
ARG TARGETPLATFORM
ARG BINARY_NAME=poz
RUN --mount=type=bind,source=artifact,target=artifact \
    if [ "$TARGETPLATFORM" = 'linux/amd64' ]; then\
      cp artifact/x86_64-unknown-linux-musl/"$BINARY_NAME" .\
    ;elif [ "$TARGETPLATFORM" = 'linux/arm64' ]; then\
      cp artifact/aarch64-unknown-linux-musl/"$BINARY_NAME" .\
    ;fi

FROM scratch
COPY --from=env --chmod=755 poz .
ENTRYPOINT ["./poz"]
