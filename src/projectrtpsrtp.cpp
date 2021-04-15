

#include <cstring>
#include <iostream>
#include <cstdlib>

#include "projectrtpsrtp.h"

gnutls_certificate_credentials_t xcred;

const char *pemfile = "dtls-srtp.pem";

/*

dnf install gnutls-devel

Note to me:

I think this is true of RTP also - we need to set the flag don't fragment
setsockopt(listen_sd, IPPROTO_IP, IP_DONTFRAG,
                           (const void *) &optval, sizeof(optval));

Notes on how this is working.

Docs are really good for gnutls. I am concerned that this is lesser gnu license, but it suites our needs at the moment.
https://www.gnutls.org/manual/gnutls.html

To use gnutls in async mode we have to use the pull, push, timeout functions (which we provide) and have to set
and return approtpriate values. As we receive data then I have to now wire this up so the call to
pull function. I need to place data in our object so that this pull function can then pull it back in.
When we have written data into our object, then if we are still performing a handshake, then
we call gnutls_handshake again to ensure the pull function is called then processed.
*/

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
    std::cerr << "Unable to et key material" << std::endl;
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

static void dtlsinit( void )
{
  gnutls_global_init();

  /* X509 stuff */
  gnutls_certificate_allocate_credentials( &xcred );

  /* sets the system trusted CAs for Internet PKI */
  gnutls_certificate_set_x509_system_trust( xcred );

  gnutls_certificate_set_x509_crl_file( xcred,
                                            pemfile,
                                            GNUTLS_X509_FMT_PEM );

  if( gnutls_certificate_set_x509_key_file( xcred,
                                              pemfile,
                                              pemfile,
                                              GNUTLS_X509_FMT_PEM ) < 0 )
  {
    std::cerr << "No certificate or key were found - quiting" << std::endl;
    exit( 1 );
  }

  gnutls_certificate_set_known_dh_params( xcred, GNUTLS_SEC_PARAM_MEDIUM );

}

static void dtlsdestroy( void )
{
  gnutls_global_deinit();
}


void dtlstest( void )
{
  dtlsinit();

  dtlssession clientsession( dtlssession::act );
  dtlssession serversession( dtlssession::pass );

  clientsession.ondata( [ &serversession ] ( const void *d , size_t l ) -> void {
    std::cout << "clientsession.ondata:" << l << std::endl;
    serversession.write( d, l );
  } );

  serversession.ondata( [ &clientsession ] ( const void *d , size_t l ) -> void {
    std::cout << "serversession.ondata:" << l << std::endl;
    clientsession.write( d, l );
  } );

  int retval;
  do
  {
    clientsession.handshake();
    retval = serversession.handshake();
  } while( GNUTLS_E_AGAIN == retval );

  if( 0 == retval )
  {
    std::cout << "TLS session negotiated" << std::endl;
    serversession.getkeys();
    clientsession.getkeys();
  }
  std::cout << +retval << std::endl;
  dtlsdestroy();

}
