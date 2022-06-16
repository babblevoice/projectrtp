
# docker build . -t <your username>/projectrtp

FROM node:18-alpine as builder

RUN npm -g install node-gyp

WORKDIR /usr/src/projectrtp
COPY . .

#libc6-compat
RUN apk add --no-cache \
    alpine-sdk cmake python3 spandsp-dev tiff-dev gnutls-dev libsrtp-dev cmake boost-dev; \
    git submodule update --init --recursive

WORKDIR /usr/src/projectrtp/libilbc
RUN cmake . -DCMAKE_INSTALL_LIBDIR=/lib -DCMAKE_INSTALL_INCLUDEDIR=/usr/include; cmake --build .; cmake --install .

# build projectrtp addon
WORKDIR /usr/src/projectrtp
RUN npm run rebuild

# Extract only the files we need to keep the final image small
RUN mkdir -p /usr/local/lib/node_modules/projectrtp/src/build/Release/ && \
    mkdir -p /usr/local/lib/node_modules/projectrtp/examples/ && \
    mkdir -p /usr/local/lib/node_modules/projectrtp/stress/ && \
    mkdir -p /usr/local/lib/node_modules/projectrtp/test/ && \
    mkdir -p /usr/local/lib/node_modules/projectrtp/lib/ && \
    cp LICENSE /usr/local/lib/node_modules/projectrtp/ && \
    cp README.md /usr/local/lib/node_modules/projectrtp/ && \
    cp index.js /usr/local/lib/node_modules/projectrtp/ && \
    cp package-lock.json /usr/local/lib/node_modules/projectrtp/ && \
    cp package.json /usr/local/lib/node_modules/projectrtp/ && \
    cp -r examples/ /usr/local/lib/node_modules/projectrtp/examples/ && \
    cp -r stress/ /usr/local/lib/node_modules/projectrtp/ && \
    cp -r test/ /usr/local/lib/node_modules/projectrtp/ && \
    cp -r lib/ /usr/local/lib/node_modules/projectrtp/ && \
    cp src/*.cpp /usr/local/lib/node_modules/projectrtp/src/ && \
    cp src/*.h /usr/local/lib/node_modules/projectrtp/src/ && \
    cp src/makefile /usr/local/lib/node_modules/projectrtp/src/ && \
    cp src/binding.gyp /usr/local/lib/node_modules/projectrtp/src/ && \
    cp -r src/build/Release/projectrtp.node /usr/local/lib/node_modules/projectrtp/src/build/Release/

WORKDIR /usr/local/lib/node_modules/projectrtp/
RUN npm ci

FROM node:18-alpine as app

RUN apk add --no-cache \
    spandsp tiff gnutls libsrtp libc6-compat openssl ca-certificates

COPY --from=builder [ "/usr/local/lib/node_modules/projectrtp/", "/usr/local/lib/node_modules/projectrtp/" ]
COPY --from=builder /lib/libilbc* /lib/

ENV NODE_PATH=/usr/local/lib/node_modules

EXPOSE 10000-20000

WORKDIR /usr/local/lib/node_modules/projectrtp/
CMD [ "node", "examples/simplenode.js" ]

