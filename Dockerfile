FROM debian:12.6-slim

ENV DEBIAN_FRONTEND=noninteractive \
    LOOS_DATA=/tmp/loos-data \
    OUT_ROOT=/tmp/loos-out \
    SRC_ROOT=/tmp/loos-src
WORKDIR /workspace

RUN apt-get update \
    && apt-get install --no-install-recommends -y \
       clang cmake ninja-build python3 \
       gcc-mingw-w64-x86-64 binutils-mingw-w64-x86-64 \
    && rm -rf /var/lib/apt/lists/*

COPY . /workspace

RUN cmake --preset host-clang \
    && cmake --build --preset host-clang \
    && cmake --preset windows-mingw \
    && cmake --build --preset windows-mingw --target locmd lonotepad lotaskmgr

CMD ["./loos", "build"]
