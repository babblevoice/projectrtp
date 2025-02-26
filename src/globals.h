#ifndef PROJECTRTPGLOBALS_H
#define PROJECTRTPGLOBALS_H

#include <boost/asio.hpp>
#include <string>
#include <memory>

/* The number of bytes in a packet ( these figure are less some overhead G711 = 172*/
#define G711PAYLOADBYTES 160
#define G722PAYLOADBYTES 160
#define L16PAYLOADSAMPLES 160
#define G711PAYLOADSAMPLES 160
#define G722PAYLOADSAMPLES 160
#define L1616PAYLOADSAMPLES 320
#define L16NARROWBANDBYTES 320
#define L16WIDEBANDBYTES 640
#define ILBC20PAYLOADBYTES 38
#define ILBC20PAYLOADSAMPLES 38 /* correct ? */
#define ILBC30PAYLOADBYTES 50 /* not needed but for completness */

#define PCMUPAYLOADTYPE 0
#define PCMAPAYLOADTYPE 8
#define G722PAYLOADTYPE 9
/* defaults for our supported dynamic payload types */
#define RFC2833PAYLOADTYPE 101
#define ILBCPAYLOADTYPE 97
/* Only use this value for internal use and must not clash with the types above */
#define L168KPAYLOADTYPE 11
#define L1616KPAYLOADTYPE 12

/* Need to double check max RTP length with variable length header - there
could be a larger length with our CODECs */

/* MAX RTP Length - MTU of UDP */
#define RTPMAXLENGTH 1500
#define RTCPMAXLENGTH 200

/*
Used to hide shared pointers so we can pass a void * into libraries tht need it.
*/
class hiddensharedptr {
public:
  hiddensharedptr( const std::shared_ptr< void >& p ): d ( nullptr ) { this->d = p; }

  template<typename T>
  auto get() {
   return std::static_pointer_cast<T>( this->d );
  }

private:
  std::shared_ptr< void > d; // Copied from Request
};

/* Use spin locks for effiencent protection and ensure unlock */
struct SpinLockGuard {
  std::atomic_bool& lock;

  SpinLockGuard( std::atomic_bool& l ) : lock( l ) {
    while( lock.exchange( true, std::memory_order_acquire ) ) {
      // Spin until the lock is acquired
    }
  }

  ~SpinLockGuard() {
    lock.store(false, std::memory_order_release);
  }
};

#define releasespinlock( x ) x.store( false, std::memory_order_release );

#endif /* PROJECTRTPGLOBALS_H */
