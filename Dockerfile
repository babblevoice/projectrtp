
# docker build . -t <your username>/node-web-app

#FROM node:16-bullseye-slim
FROM node:16-alpine as builder

WORKDIR /usr/local/lib/node_modules/projectrtp
COPY . .

RUN apk add --no-cache \
    --virtual .gyp python3 make g++ boost-dev \
    spandsp-dev tiff-dev gnutls-dev libsrtp-dev libc6-compat git cmake abseil-cpp

WORKDIR /usr/src/
RUN wget https://github.com/TimothyGu/libilbc/releases/download/v3.0.4/libilbc-3.0.4.tar.gz; \
    tar xvzf libilbc-3.0.4.tar.gz; \
    cd libilbc-3.0.4; \
    cmake . -DCMAKE_INSTALL_LIBDIR=/lib -DCMAKE_INSTALL_INCLUDEDIR=/usr/include; cmake --build .; cmake --install .

# build projectrtp addon
WORKDIR /usr/local/lib/node_modules/projectrtp/src
RUN /usr/local/lib/node_modules/npm/bin/node-gyp-bin/node-gyp rebuild

FROM node:16-alpine as app

RUN apk add --no-cache \
    spandsp tiff gnutls libsrtp libc6-compat openssl ca-certificates
# ilbc

WORKDIR /usr/src/app
COPY --from=builder /usr/local/lib/node_modules/projectrtp/examples/* ./
COPY --from=builder /usr/local/lib/node_modules/projectrtp /usr/local/lib/node_modules/projectrtp
COPY --from=builder /lib/libilbc* /lib/

RUN npm install /usr/local/lib/node_modules/projectrtp/

EXPOSE 10000-2000
CMD [ "node", "simplenode.js" ]

