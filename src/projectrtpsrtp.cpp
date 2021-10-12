
#include <sstream>
#include <cstring>
#include <iostream>
#include <cstdlib>
#include <iomanip>

/* Hash functions */
#include <gnutls/crypto.h>
#include <gnutls/x509.h>

/* ntohs etc */
#include <arpa/inet.h>

/* strol etc */
#include <stdlib.h>

/* min etc */
#include <algorithm>

#include "projectrtpsrtp.h"

static gnutls_certificate_credentials_t xcred;
static std::string fingerprintsha256;

const char *pemfile = "dtls-srtp.pem";


#ifdef DTLSDEBUGOUTPUT
static void serverlogfunc(int level, const char *str)
{
  std::cerr << +level << ":" << str << std::endl;
}
#endif

dtlssession::dtlssession( dtlssession::mode mode ) :
  m( mode ),
  inindex( 0 ),
  incount( 0 )
{
  if( dtlssession::act == mode )
  {
    gnutls_init( &this->session, GNUTLS_CLIENT | GNUTLS_DATAGRAM | GNUTLS_NONBLOCK );
  }
  else
  {
    gnutls_init( &this->session, GNUTLS_SERVER | GNUTLS_DATAGRAM | GNUTLS_NONBLOCK );
    gnutls_certificate_server_set_request( this->session, GNUTLS_CERT_REQUIRE );

    gnutls_datum_t skey;

    gnutls_session_ticket_key_generate( &skey );
    gnutls_session_ticket_enable_server( this->session, &skey );
    gnutls_free( skey.data );
  }

#ifdef DTLSDEBUGOUTPUT
  gnutls_global_set_log_function( serverlogfunc );
  gnutls_global_set_log_level( 4711 );
#endif

  gnutls_set_default_priority( this->session );
  gnutls_credentials_set( this->session, GNUTLS_CRD_CERTIFICATE, xcred );

  gnutls_dtls_set_mtu( this->session, DTLSMTUSIZE );

  gnutls_srtp_set_profile( this->session, GNUTLS_SRTP_AES128_CM_HMAC_SHA1_80 ); // or maybe GNUTLS_SRTP_AES128_CM_HMAC_SHA1_32?
  gnutls_transport_set_ptr( this->session, this );

  gnutls_dtls_set_timeouts( this->session, 1000, 6 * 1000 );

  gnutls_transport_set_push_function( this->session, [] ( gnutls_transport_ptr_t p, const void *data, size_t size ) -> ssize_t {
    ( ( dtlssession * )( p ) )->push( data, size );
    return size;
  } );

  gnutls_transport_set_pull_function( this->session, [] ( gnutls_transport_ptr_t p, void *data, size_t size ) -> ssize_t {
    return ( ( dtlssession * )( p ) )->pull( data, size );
  } );

  gnutls_transport_set_pull_timeout_function( this->session, [] ( gnutls_transport_ptr_t p, unsigned int ms ) -> int {
    return ( ( dtlssession * )( p ) )->timeout( ms );
  } );
}

dtlssession::~dtlssession()
{
  gnutls_deinit( this->session );
}

dtlssession::pointer dtlssession::create( dtlssession::mode mode )
{
  return pointer( new dtlssession( mode ) );
}

void dtlssession::push( const void *data, size_t size )
{
  this->bindwritefunc( data, size );

#ifdef DTLSDEBUGOUTPUT
  DTLSPlaintext *tmp = ( DTLSPlaintext* ) data;

  std::cout << "type 22 = handshake. major.minor = 254.253 for TLS 1.2" << std::endl;
  std::cout << "DTLSPlaintext->type:" << +tmp->type << std::endl;
  std::cout << "DTLSPlaintext->version->major:" << +tmp->version.major << std::endl;
  std::cout << "DTLSPlaintext->version->minor:" << +tmp->version.minor << std::endl;
  std::cout << "DTLSPlaintext->epoch:" << +tmp->epoch << std::endl;
  std::cout << "DTLSPlaintext->sequencenumber:" << +tmp->sequencenumber << std::endl;
  std::cout << "DTLSPlaintext->length:" << +tmp->length << std::endl;
#endif

  //gnutls_transport_set_errno( this->session, EAGAIN );
}

int dtlssession::pull( void *data, size_t size )
{
  if( 0 == this->incount )
  {
    gnutls_transport_set_errno( this->session, EAGAIN );
    return -1;
  }

  auto ind = ( this->inindex - this->incount + DTLSNUMBUFFERS ) % DTLSNUMBUFFERS;
  auto insz = this->insize[ ind ];

  if( size > insz )
  {
    std::memcpy( data, ( void * ) this->indata[ ind ], insz );
    this->incount--;
    return insz;
  }

  std::cerr << "Problem with DTLS buffers" << std::endl;
  return 0; /* indicate termination */

}

int dtlssession::timeout( unsigned int ms )
{
  if( this->incount > 0 )
  {
    /* when we receive data we return a positive number then gnutls will call our pull function */
    return 1;
  }

  if( 0 == ms ) return 0;

  gnutls_transport_set_errno( this->session, EAGAIN );
  return -1;
}


int dtlssession::handshake( void )
{
  return gnutls_handshake( this->session );
}

void dtlssession::write( const void *data, size_t size )
{
  if( this->incount > DTLSNUMBUFFERS )
  {
    std::cerr << "Not enough buffers for DTLS negotiation" << std::endl;
    return;
  }

  std::memcpy( ( void * ) this->indata[ this->inindex ], data, size );
  this->insize[ this->inindex ] = size;

  this->inindex = ( this->inindex + 1 )  % DTLSNUMBUFFERS;
  this->incount++;
}

/*
ref: https://gitlab.com/gnutls/gnutls/blob/master/tests/mini-dtls-srtp.c
*/
void dtlssession::getkeys( void )
{
  std::cout << gnutls_protocol_get_name( gnutls_protocol_get_version( this->session ) ) << std::endl;

  uint8_t km[ DTLSMAXKEYMATERIAL ];
  gnutls_datum_t srtp_cli_key, srtp_cli_salt, srtp_server_key, srtp_server_salt;

  if( gnutls_srtp_get_keys(session, km, sizeof( km ), &srtp_cli_key, &srtp_cli_salt,
                        &srtp_server_key, &srtp_server_salt) < 0 )
  {
    std::cerr << "Unable to get key material" << std::endl;
    return;
  }
  //gnutls_srtp_get_keys( this->session, )
  char buf[ 2 * DTLSMAXKEYMATERIAL ];
  size_t size = sizeof( buf );
  gnutls_hex_encode( &srtp_cli_key, buf, &size );
  std::cout << "Client key: " << buf << std::endl;

  size = sizeof(buf);
  gnutls_hex_encode( &srtp_cli_salt, buf, &size );
  std::cout << "Client salt: " << buf << std::endl;

  size = sizeof(buf);
  gnutls_hex_encode( &srtp_server_key, buf, &size );
  std::cout << "Server key: " << buf << std::endl;

  size = sizeof(buf);
  gnutls_hex_encode( &srtp_server_salt, buf, &size );
  std::cout << "Server salt: " << buf << std::endl;
}

int dtlssession::peersha256( void )
{
  unsigned int l;
  const gnutls_datum_t *c = gnutls_certificate_get_peers( this->session, &l );
  uint8_t digest[ 32 ];

  if( nullptr != c && l > 0 && c[ 0 ].size > 0 )
  {
    gnutls_hash_fast( GNUTLS_DIG_SHA256, ( const void * ) c->data, c->size, ( void * ) digest );

    if( 0 == memcmp( digest, peersha256sum, 32 ) )
    {
      std::cout << "hello: compared ok!!!" << std::endl;
      return 0;
    }
  }
  return -1;
}

/* Takes the format: 70:4B:FC:94:C8:41:C4:A3:54:96:8A:DD:6C:FD:CD:20:77:45:82:B7:F2:45:F5:79:81:D9:BB:FB:A7:3A:5A:C4
and converts to a char array and sotres for checking. */
void dtlssession::setpeersha256( std::string &s )
{
  auto n = std::min( (int) ( s.size() + 1 ) / 3, 32 );
  char conv[ 3 ];
  conv[ 2 ] = 0;
  for( auto i = 0; i < n; i ++ )
  {
    conv[ 0 ] = s[ i * 3 ];
    conv[ 1 ] = s[ ( i *3 ) + 1 ];
    this->peersha256sum[ i ] = strtol( conv, NULL, 16);
  }
}


/*
TODO At the moment we load a cert from a file. I would prefer to generate this on the fly on startup.
*/
static void dtlsinit( void )
{
  gnutls_global_init();

  /* X509 stuff */
  gnutls_certificate_allocate_credentials( &xcred );
  gnutls_certificate_set_flags( xcred, GNUTLS_CERTIFICATE_API_V2 );

  /* sets the system trusted CAs for Internet PKI */
  gnutls_certificate_set_x509_system_trust( xcred );

  gnutls_certificate_set_x509_crl_file( xcred,
                                            pemfile,
                                            GNUTLS_X509_FMT_PEM );

  int idx;
  if( ( idx = gnutls_certificate_set_x509_key_file( xcred,
                                              pemfile,
                                              pemfile,
                                              GNUTLS_X509_FMT_PEM ) ) < 0 )
  {
    std::cerr << "No certificate or key were found - quiting" << std::endl;
    exit( 1 );
  }

  gnutls_certificate_set_known_dh_params( xcred, GNUTLS_SEC_PARAM_MEDIUM );

  gnutls_certificate_set_verify_function( xcred, [] ( gnutls_session_t s ) -> int {
    std::cout << "gnutls_certificate_set_verify_function" << std::endl;
    return ( ( dtlssession * ) gnutls_transport_get_ptr( s ) )->peersha256();
  } );

  /* Calculate our fingerprint of our cert we pass over to our peer (this is a shasum of our public DER cert) */
  gnutls_x509_crt_t *crts;
	unsigned ncrts;
  gnutls_datum_t crtdata;

  if( GNUTLS_E_SUCCESS != gnutls_certificate_get_x509_crt( xcred, idx, &crts, &ncrts ) )
  {
    std::cerr << "Problem gettin our DER cert" << std::endl;
    exit( 1 );
  }

  uint8_t digest[ 32 ];
  for ( unsigned int i = 0; i < ncrts; i++ )
  {
    gnutls_x509_crt_export2( crts[ i ],
						                  GNUTLS_X509_FMT_DER,
						                  &crtdata );

    gnutls_hash_fast( GNUTLS_DIG_SHA256, ( const void * ) crtdata.data, crtdata.size, ( void * ) digest );
    gnutls_x509_crt_deinit( crts[ i ] );
    gnutls_free( crtdata.data );
  }

  gnutls_free( crts );

  /* Convert to the string view which is needed for SDP (a=fingerprint:sha-256 ...) */
  std::stringstream conv;
  conv << std::hex << std::setfill( '0' );
  for( unsigned int i = 0; i < 31; i++ )
  {
    conv << std::setw( 2 ) << std::uppercase << ( int ) digest[ i ] << ":";
  }
  conv << std::setw( 2 ) << std::uppercase << ( int ) digest[ 31 ];
  fingerprintsha256 = conv.str();
}

static void dtlsdestroy( void )
{
  gnutls_certificate_free_credentials( xcred );
  gnutls_global_deinit();
}

#ifdef TESTSUITE
void dtlstest( void )
{
  dtlsinit();

  std::cout << "a=fingerprint:sha-256 " << fingerprintsha256 << std::endl;

  dtlssession::pointer clientsession = dtlssession::create( dtlssession::act );
  dtlssession::pointer serversession = dtlssession::create( dtlssession::pass );

  clientsession->setpeersha256( fingerprintsha256 );
  serversession->setpeersha256( fingerprintsha256 );

  clientsession->ondata( [ &serversession ] ( const void *d , size_t l ) -> void {
    std::cout << "clientsession.ondata:" << l << std::endl;
    serversession->write( d, l );
  } );

  serversession->ondata( [ &clientsession ] ( const void *d , size_t l ) -> void {
    std::cout << "serversession.ondata:" << l << std::endl;
    clientsession->write( d, l );
  } );

  int retval;
  do
  {
    retval = clientsession->handshake();
    serversession->handshake();
  } while( GNUTLS_E_AGAIN == retval );

  if( 0 == retval )
  {
    std::cout << "TLS session negotiated" << std::endl;
    serversession->getkeys();
    clientsession->getkeys();
  }

  dtlsdestroy();

}
#endif

const char* getdtlssrtpsha256fingerprint( void )
{
  return fingerprintsha256.c_str();
}
