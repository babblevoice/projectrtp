# Pinned to 1.92 — rust 1.94.1 on musl ICEs during borrow-check of
# rustls 0.23.38 (pulled in transitively by webrtc-dtls / webrtc-srtp).
# Re-evaluate when we move to an rustls that avoids the offending pattern
# or when a rustc 1.95+ musl image lands with the fix.
FROM rust:1.92-alpine3.22 AS rust-builder
WORKDIR /src
# Alpine 3.22 doesn't ship an iLBC package, so we build the bundled WebRTC
# libilbc source tree ourselves and install it into /usr/local. cmake +
# build-base gets us g++/make; libstdc++ is already in the rust image via
# the toolchain.
RUN apk add --no-cache musl-dev pkgconfig cmake build-base linux-headers spandsp3-dev
COPY libilbc/ /src/libilbc/
RUN mkdir -p /src/libilbc/_build && \
    cd /src/libilbc/_build && \
    cmake -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr/local .. && \
    make -j"$(nproc)" && make install && \
    ldconfig /usr/local/lib 2>/dev/null || true
COPY rust/ /src/rust/
WORKDIR /src/rust
# Alpine's rust defaults target musl, which forces `-C target-feature=+crt-static`.
# A cdylib (.so loaded by Node at runtime) needs dynamic linkage, so disable it.
ENV RUSTFLAGS="-C target-feature=-crt-static"
RUN cargo build --release                                                                                                                                                                   
                                                                                                                                                                                              
FROM alpine:3.22 AS test
WORKDIR /usr/src/projectrtp
RUN apk add --no-cache nodejs npm ca-certificates libstdc++ spandsp3
# Copy libilbc out of the builder stage — Alpine doesn't have a package
# for it, and the Rust cdylib needs the .so at Node-load time. spandsp
# is a packaged runtime (above), so no manual copy needed.
COPY --from=rust-builder /usr/local/lib/libilbc.so* /usr/lib/
COPY . .                                                                                                                                                                                    
COPY --from=rust-builder /src/rust/target/release/libprojectrtp.so ./build/Release/projectrtp.node
RUN npm ci --ignore-scripts   # native lib comes from the rust-builder stage; no build scripts needed                                                                                                                    
CMD [ "./node_modules/mocha/bin/_mocha", "test/interface/*.js", "test/unit/*.js", "--exit" ]                                                                                                
                                                                                                                                                                                              
FROM alpine:3.22 AS app
WORKDIR /app
RUN apk add --no-cache nodejs npm ca-certificates libstdc++ spandsp3
COPY --from=rust-builder /usr/local/lib/libilbc.so* /usr/lib/
COPY --from=rust-builder /src/rust/target/release/libprojectrtp.so /app/build/Release/projectrtp.node
COPY index.js package.json package-lock.json /app/                                                                                                                                                            
COPY lib/ /app/lib/                                       
COPY examples/ /app/examples/                                                                                                                                                               
RUN npm ci --omit=dev --ignore-scripts                    
EXPOSE 10000-50000/udp                                                                                                                                                                      
CMD [ "node", "examples/simplenode.js" ]
