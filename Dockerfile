
# docker build . -t <your username>/projectrtp
# I have tied this to version alpine 3.16 as 3.17 has an exception symbol dynamic linking issue.
FROM alpine:3.20 AS builder


WORKDIR /usr/src/

RUN apk add --no-cache \
    alpine-sdk cmake python3 spandsp-dev tiff-dev gnutls-dev libsrtp-dev cmake boost-dev nodejs npm; \
    npm -g install node-gyp; \
    wget https://github.com/TimothyGu/libilbc/releases/download/v3.0.4/libilbc-3.0.4.tar.gz; \
    tar xvzf libilbc-3.0.4.tar.gz; \
    cd libilbc-3.0.4; \
    cmake . -DCMAKE_INSTALL_LIBDIR=/lib -DCMAKE_INSTALL_INCLUDEDIR=/usr/include; cmake --build .; cmake --install .; 
    
WORKDIR /usr/local/lib/node_modules/@babblevoice/projectrtp/
COPY . .

RUN npm ci --no-optional --production;\
    rm -fr src/build/Release/obj.target

FROM alpine:3.20 AS app

RUN apk add --no-cache \
    spandsp tiff gnutls libsrtp libc6-compat openssl ca-certificates nodejs npm

COPY --from=builder /usr/local/lib/node_modules/@babblevoice/projectrtp/ /usr/local/lib/node_modules/@babblevoice/projectrtp/
COPY --from=builder /lib/libilbc* /lib/

ENV NODE_PATH=/usr/local/lib/node_modules

EXPOSE 10000-50000/udp

WORKDIR /usr/local/lib/node_modules/@babblevoice/projectrtp/
CMD [ "node", "examples/simplenode.js" ]