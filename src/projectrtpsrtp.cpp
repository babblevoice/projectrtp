
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
static void serverlogfunc(int level, const char *str) {
  std::cerr << +level << ":" << str << std::endl;
}
#endif

dtlssession::dtlssession( dtlssession::mode mode ) :
  m( mode ),
  inindex( 0 ),
  incount( 0 ),
  keymaterial(),
  srtsendppolicy(),
  srtrecvppolicy(),
  srtpsendsession( nullptr ),
  srtprecvsession( nullptr ) {
  if( dtlssession::act == mode ) {
    gnutls_init( &this->session, GNUTLS_CLIENT | GNUTLS_DATAGRAM | GNUTLS_NONBLOCK );
  } else {
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

  /* srtp */
  memset( &this->srtsendppolicy, 0x0, sizeof( srtp_policy_t ) );
  memset( &this->srtrecvppolicy, 0x0, sizeof( srtp_policy_t ) );
  srtp_crypto_policy_set_rtp_default( &this->srtsendppolicy.rtp );
  srtp_crypto_policy_set_rtcp_default( &this->srtsendppolicy.rtcp );
  srtp_crypto_policy_set_rtp_default( &this->srtrecvppolicy.rtp );
  srtp_crypto_policy_set_rtcp_default( &this->srtrecvppolicy.rtcp );

  memset( this->keymaterial, 0x0, DTLSMAXKEYMATERIAL );
}

dtlssession::~dtlssession() {
  gnutls_deinit( this->session );

  if( nullptr != this->srtpsendsession ) srtp_dealloc( this->srtpsendsession );
  if( nullptr != this->srtprecvsession ) srtp_dealloc( this->srtprecvsession );
}

dtlssession::pointer dtlssession::create( dtlssession::mode mode ) {
  return pointer( new dtlssession( mode ) );
}

void dtlssession::push( const void *data, size_t size ) {
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

int dtlssession::pull( void *data, size_t size ) {
  if( 0 == this->incount ) {
    gnutls_transport_set_errno( this->session, EAGAIN );
    return -1;
  }

  auto ind = ( this->inindex - this->incount + DTLSNUMBUFFERS ) % DTLSNUMBUFFERS;
  auto insz = this->insize[ ind ];

  if( size > insz ) {
    std::memcpy( data, ( void * ) this->indata[ ind ], insz );
    this->incount--;
    return insz;
  }

  fprintf( stderr, "Problem with DTLS buffers\n" );
  return 0; /* indicate termination */

}

int dtlssession::timeout( unsigned int ms ) {
  if( this->incount > 0 ) {
    /* when we receive data we return a positive number then gnutls will call our pull function */
    return 1;
  }

  if( 0 == ms ) return 0;

  gnutls_transport_set_errno( this->session, EAGAIN );
  return -1;
}


int dtlssession::handshake( void ) {
  auto retval = gnutls_handshake( this->session );
  if( GNUTLS_E_AGAIN == retval ) return GNUTLS_E_AGAIN;
  if( 0 == retval ) this->getkeys();
  return retval;
}

void dtlssession::write( const void *data, size_t size ) {
  if( this->incount > DTLSNUMBUFFERS ) {
    fprintf( stderr, "Not enough buffers for DTLS negotiation\n" );
    return;
  }

  std::memcpy( ( void * ) this->indata[ this->inindex ], data, size );
  this->insize[ this->inindex ] = size;

  this->inindex = ( this->inindex + 1 )  % DTLSNUMBUFFERS;
  this->incount++;
}

/*
Load our keys into our policies.
ref: https://gitlab.com/gnutls/gnutls/blob/master/tests/mini-dtls-srtp.c
*/
void dtlssession::getkeys( void ) {
#ifdef DTLSDEBUGOUTPUT
  std::cout << gnutls_protocol_get_name( gnutls_protocol_get_version( this->session ) ) << std::endl;
#endif

  gnutls_datum_t srtp_cli_key, srtp_cli_salt, srtp_server_key, srtp_server_salt;

  if( gnutls_srtp_get_keys( this->session, this->keymaterial, DTLSMAXKEYMATERIAL, &srtp_cli_key, &srtp_cli_salt,
                        &srtp_server_key, &srtp_server_salt ) < 0 ) {
    fprintf( stderr, "Unable to get key material\n" );
    return;
  }

  if( 32 == srtp_cli_key.size ) {
    srtp_crypto_policy_set_aes_gcm_256_8_only_auth( &this->srtsendppolicy.rtp );
    srtp_crypto_policy_set_aes_gcm_256_8_only_auth( &this->srtsendppolicy.rtcp );
  } else {
    srtp_crypto_policy_set_aes_gcm_128_8_only_auth( &this->srtsendppolicy.rtp );
    srtp_crypto_policy_set_aes_gcm_128_8_only_auth( &this->srtsendppolicy.rtcp );
  }

  if( 32 == srtp_server_key.size ) {
    srtp_crypto_policy_set_aes_gcm_256_8_only_auth( &this->srtrecvppolicy.rtp );
    srtp_crypto_policy_set_aes_gcm_256_8_only_auth( &this->srtrecvppolicy.rtcp );
  } else {
    srtp_crypto_policy_set_aes_gcm_128_8_only_auth( &this->srtrecvppolicy.rtp );
    srtp_crypto_policy_set_aes_gcm_128_8_only_auth( &this->srtrecvppolicy.rtcp );
  }

  if( dtlssession::act == this->m ) {
    this->srtsendppolicy.ssrc.type = ssrc_any_outbound; // or ssrc_specific?
    this->srtsendppolicy.key = srtp_server_key.data;

    this->srtrecvppolicy.ssrc.type = ssrc_any_inbound;
    this->srtrecvppolicy.key = srtp_cli_key.data;
  } else { /* dtlssession::pass */
    this->srtsendppolicy.ssrc.type = ssrc_any_inbound;
    this->srtsendppolicy.key = srtp_cli_key.data;

    this->srtrecvppolicy.ssrc.type = ssrc_any_outbound;
    this->srtrecvppolicy.key = srtp_server_key.data;
  }

  if( srtp_create( &this->srtpsendsession, &this->srtsendppolicy ) ) {
    fprintf( stderr, "Unable create sending srtp session\n" );
    return;
  }

  if( srtp_create( &this->srtprecvsession, &this->srtrecvppolicy ) ) {
    fprintf( stderr, "Unable create receiving srtp session\n" );
    return;
  }

#ifdef DTLSDEBUGOUTPUT
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
#endif
}

int dtlssession::peersha256( void ) {
  unsigned int l;
  const gnutls_datum_t *c = gnutls_certificate_get_peers( this->session, &l );
  uint8_t digest[ 32 ];

  if( nullptr != c && l > 0 && c[ 0 ].size > 0 ) {
    gnutls_hash_fast( GNUTLS_DIG_SHA256, ( const void * ) c->data, c->size, ( void * ) digest );

    if( 0 == memcmp( digest, peersha256sum, 32 ) ) {
      //std::cout << "hello: compared ok!!!" << std::endl;
      return 0;
    }
  }
  return -1;
}

/* Takes the format: 70:4B:FC:94:C8:41:C4:A3:54:96:8A:DD:6C:FD:CD:20:77:45:82:B7:F2:45:F5:79:81:D9:BB:FB:A7:3A:5A:C4
and converts to a char array and sotres for checking. */
void dtlssession::setpeersha256( std::string &s ) {
  auto n = std::min( (int) ( s.size() + 1 ) / 3, 32 );
  char conv[ 3 ];
  conv[ 2 ] = 0;
  for( auto i = 0; i < n; i ++ ) {
    conv[ 0 ] = s[ i * 3 ];
    conv[ 1 ] = s[ ( i *3 ) + 1 ];
    this->peersha256sum[ i ] = strtol( conv, NULL, 16);
  }
}

/* do we need to support mki? */
bool dtlssession::protect( rtppacket *pk ) {

  int length = pk->length;

  auto stat = srtp_protect( this->srtpsendsession, pk->pk, &length );
  if( srtp_err_status_ok != stat ) {
    fprintf( stderr, "Error: srtp protect failed with code %d\n", stat );
    return false;
  }
  pk->length = length;
  return true;
}

bool dtlssession::unprotect( rtppacket *pk ) {

  int length = pk->length;
  auto stat = srtp_unprotect( this->srtprecvsession, pk->pk, &length );
  if( srtp_err_status_ok != stat ) {
    fprintf( stderr, "Error: srtp unprotect failed with code %d\n", stat );
    return false;
  }
  pk->length = length;
  return true;
}


/*
TODO At the moment we load a cert from a file. I would prefer to generate this on the fly on startup.
*/
static bool dtlsinitied = false;
void dtlsinit( void ) {
  if( dtlsinitied ) return;
  dtlsinitied = true;

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
                                              GNUTLS_X509_FMT_PEM ) ) < 0 ) {
    fprintf( stderr, "No certificate or key found - quiting\n" );
    exit( 1 );
  }

  gnutls_certificate_set_known_dh_params( xcred, GNUTLS_SEC_PARAM_MEDIUM );

  gnutls_certificate_set_verify_function( xcred, [] ( gnutls_session_t s ) -> int {
    //std::cout << "gnutls_certificate_set_verify_function" << std::endl;
    return ( ( dtlssession * ) gnutls_transport_get_ptr( s ) )->peersha256();
  } );

  /* Calculate our fingerprint of our cert we pass over to our peer (this is a shasum of our public DER cert) */
  gnutls_x509_crt_t *crts;
	unsigned ncrts;
  gnutls_datum_t crtdata;

  if( GNUTLS_E_SUCCESS != gnutls_certificate_get_x509_crt( xcred, idx, &crts, &ncrts ) ) {
    fprintf( stderr, "Problem gettin our DER cert\n" );
    exit( 1 );
  }

  uint8_t digest[ 32 ];
  for ( unsigned int i = 0; i < ncrts; i++ ) {
    gnutls_x509_crt_export2( crts[ i ], GNUTLS_X509_FMT_DER, &crtdata );
    gnutls_hash_fast( GNUTLS_DIG_SHA256, ( const void * ) crtdata.data, crtdata.size, ( void * ) digest );
    gnutls_x509_crt_deinit( crts[ i ] );
    gnutls_free( crtdata.data );
  }

  gnutls_free( crts );

  /* Convert to the string view which is needed for SDP (a=fingerprint:sha-256 ...) */
  std::stringstream conv;
  conv << std::hex << std::setfill( '0' );
  for( unsigned int i = 0; i < 31; i++ ) {
    conv << std::setw( 2 ) << std::uppercase << ( int ) digest[ i ] << ":";
  }
  conv << std::setw( 2 ) << std::uppercase << ( int ) digest[ 31 ];
  fingerprintsha256 = conv.str();
}

void dtlsdestroy( void ) {
  if( !dtlsinitied ) return;
  dtlsinitied = false;

  gnutls_certificate_free_credentials( xcred );
  gnutls_global_deinit();
}

#ifdef TESTSUITE
void dtlstest( void ) {
  dtlsinit();

#ifdef DTLSDEBUGOUTPUT
  std::cout << "a=fingerprint:sha-256 " << fingerprintsha256 << std::endl;
#endif

  dtlssession::pointer clientsession = dtlssession::create( dtlssession::act );
  dtlssession::pointer serversession = dtlssession::create( dtlssession::pass );

  clientsession->setpeersha256( fingerprintsha256 );
  serversession->setpeersha256( fingerprintsha256 );

  /* just loop back the 2 sessions to each other */
  clientsession->ondata( [ &serversession ] ( const void *d , size_t l ) -> void {
    //std::cout << "clientsession.ondata:" << l << std::endl;
    serversession->write( d, l );
  } );

  serversession->ondata( [ &clientsession ] ( const void *d , size_t l ) -> void {
    //std::cout << "serversession.ondata:" << l << std::endl;
    clientsession->write( d, l );
  } );

  int retval;
  do {
    retval = clientsession->handshake();
    serversession->handshake();
  } while( GNUTLS_E_AGAIN == retval );

  if( 0 == retval ) {
#ifdef DTLSDEBUGOUTPUT
    std::cout << "DTLS session negotiated" << std::endl;
#endif
  } else {
    throw "Failed to negotiate TLS Session";
  }

  rtppacket ourpacket;

  ourpacket.setlength( 172 );
  uint8_t *pl = ourpacket.getpayload();
  pl[ 10 ] = 4;
  pl[ 20 ] = 88;
  pl[ 50 ] = 34;

#ifdef DTLSDEBUGOUTPUT
  ourpacket.dump();
#endif
  if( !clientsession->protect( &ourpacket ) ) throw "Failed to protect RTP packet";
#ifdef DTLSDEBUGOUTPUT
  ourpacket.dump();
#endif
  if( !serversession->unprotect( &ourpacket ) ) throw "Failed to unprotect RTP packet";

#ifdef DTLSDEBUGOUTPUT
  ourpacket.dump();
#endif

  pl = ourpacket.getpayload();
  if( 4 != pl[ 10 ] || 88 != pl[ 20 ] || 34 != pl[ 50 ] ) throw "We didn't unprotect our SRTP data correctly";

  dtlsdestroy();

}
#endif

const char* getdtlssrtpsha256fingerprint( void ) {
  return fingerprintsha256.c_str();
}

#ifdef NODE_MODULE
void initsrtp( napi_env env, napi_value &result ) {
  napi_value ndtls, fp;
  if( napi_ok != napi_create_object( env, &ndtls ) ) return;
  if( napi_ok != napi_set_named_property( env, result, "dtls", ndtls ) ) return;

  if( napi_ok != napi_create_string_utf8( env, getdtlssrtpsha256fingerprint(), NAPI_AUTO_LENGTH, &fp ) ) return;
  if( napi_ok != napi_set_named_property( env, ndtls, "fingerprint", fp ) ) return;
}

#endif
