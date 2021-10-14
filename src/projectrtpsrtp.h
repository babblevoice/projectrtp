
#ifndef PROJECTRTPSRPT_H
#define PROJECTRTPSRPT_H

#include <gnutls/gnutls.h>
#include <gnutls/dtls.h>

#include <srtp2/srtp.h>

#include <functional>

#include <boost/enable_shared_from_this.hpp>

#include "projectrtppacket.h"

#define DTLSMTUSIZE 1200
#define DTLSNUMBUFFERS 10

#define DTLSMAXKEYMATERIAL (64 * 4)

//#define DTLSDEBUGOUTPUT 1

struct ProtocolVersion {
  uint8_t major;
  uint8_t minor;
};


class DTLSPlaintext {
public:
  uint8_t type;
  ProtocolVersion version;
  uint16_t epoch;
  uint64_t sequencenumber:48;
  uint16_t length;
  //opaque fragment[DTLSPlaintext.length];
};


class dtlssession:
  public std::enable_shared_from_this< dtlssession > {
public:

  enum mode { act, pass };
  dtlssession( mode m );
  ~dtlssession();

  dtlssession( const dtlssession& ) = delete;              // copy ctor
  dtlssession( dtlssession&& ) = delete;                   // move ctor
  dtlssession& operator=( const dtlssession& ) = delete;   // copy assignment
  dtlssession& operator=( dtlssession&& ) = delete;        // move assignment

  typedef std::shared_ptr< dtlssession > pointer;
  static pointer create( mode m );

  int handshake( void );
  void write( const void *data, size_t size );
  void ondata( std::function< void( const void*, size_t )> f ) { this->bindwritefunc = f; }
  void getkeys( void );
  gnutls_session_t get( void ) { return this->session; }

  void setpeersha256( std::string &s );
  int peersha256( void );

  /* Private, but not private - our callback functions from our gnutls interface. */
  void push( const void *data, size_t size );
  int pull( void *data, size_t size );
  int timeout( unsigned int ms );

  bool protect( rtppacket *pk );
  bool unprotect( rtppacket *pk );

private:
  gnutls_session_t session;
  mode m;

  uint8_t indata[ DTLSNUMBUFFERS ][ DTLSMTUSIZE ];
  size_t insize[ DTLSNUMBUFFERS ];
  int inindex;
  int incount;

  std::function< void( const void*, size_t ) > bindwritefunc;

  uint8_t peersha256sum[ 32 ];

  /* srtp policy and sessions */
  uint8_t keymaterial[ DTLSMAXKEYMATERIAL ];
  srtp_policy_t srtsendppolicy;
  srtp_policy_t srtrecvppolicy;
  srtp_t srtpsendsession;
  srtp_t srtprecvsession;
};

void dtlsinit( void );
void dtlsdestroy( void );

const char* getdtlssrtpsha256fingerprint( void );

#ifdef NODE_MODULE
void initsrtp( napi_env env, napi_value &result );
#endif

#ifdef TESTSUITE
void dtlstest( void );
#endif


#endif /* PROJECTRTPSRPT_H */
