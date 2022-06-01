
# docker build . -t <your username>/projectrtp

FROM node:16-alpine as builder

RUN npm -g install node-gyp

WORKDIR /usr/local/lib/node_modules/projectrtp
COPY . .

RUN apk add --no-cache \
    python3 git make g++ boost-dev \
    spandsp-dev tiff-dev gnutls-dev libsrtp-dev libc6-compat cmake

RUN git submodule update --init --recursive

WORKDIR /usr/local/lib/node_modules/projectrtp/libilbc
RUN cmake . -DCMAKE_INSTALL_LIBDIR=/lib -DCMAKE_INSTALL_INCLUDEDIR=/usr/include; \
    cmake --build .; \
    cmake --install .

# build projectrtp addon
WORKDIR /usr/local/lib/node_modules/projectrtp/src
RUN /usr/local/lib/node_modules/npm/bin/node-gyp-bin/node-gyp rebuild

FROM node:16-alpine as app

RUN apk add --no-cache \
    spandsp tiff gnutls libsrtp libc6-compat openssl ca-certificates

RUN npm -g install node-gyp

COPY --from=builder /usr/local/lib/node_modules/projectrtp /usr/local/lib/node_modules/projectrtp
COPY --from=builder /lib/libilbc* /lib/

WORKDIR /usr/local/lib/node_modules/projectrtp
RUN npm install

EXPOSE 10000-20000
CMD [ "node", "/usr/local/lib/node_modules/projectrtp/examples/simplenode.js" ]

