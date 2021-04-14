

#include <gnutls/gnutls.h>
#include <gnutls/dtls.h>

#include <functional>


#define DTLSMTUSIZE 1200
#define DTLSNUMBUFFERS 10


//#define DTLSDEBUGOUTPUT 1

struct ProtocolVersion
{
  uint8_t major;
  uint8_t minor;
};


class DTLSPlaintext
{
public:
  uint8_t type;
  ProtocolVersion version;
  uint16_t epoch;                                    // New field
  uint64_t sequencenumber:48;                          // New field
  uint16_t length;
  //opaque fragment[DTLSPlaintext.length];
};


class dtlssession
{
public:

  enum mode { act, pass };
  dtlssession( mode m );

  int handshake( void );
  void write( const void *data, size_t size );
  void ondata( std::function< void( const void*, size_t )> f ) { this->bindwritefunc = f; }

  /* Private, but not private - our callback functions from our gnutls interface. */
  void push( const void *data, size_t size );
  int pull( void *data, size_t size );
  int timeout( unsigned int ms );

private:
  gnutls_session_t session;
  mode m;

  uint8_t indata[ DTLSNUMBUFFERS ][ DTLSMTUSIZE ];
  size_t insize[ DTLSNUMBUFFERS ];
  int inindex;
  int incount;

  std::function< void( const void*, size_t ) > bindwritefunc;
};

void dtlstest( void );
