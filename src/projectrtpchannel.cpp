

#include <iostream>
#include <cstdlib>

#include <boost/bind/bind.hpp>
#include <boost/chrono.hpp>
#include <iomanip>
#include <utility>

#include <queue>

/* gnutls_rnd */
#include <gnutls/crypto.h>

#include "projectrtpchannel.h"
#include "projectrtpstun.h"

extern boost::asio::io_context workercontext;
std::queue < unsigned short >availableports;
std::atomic_bool availableportslock( false );

std::atomic< std::uint32_t > channelscreated{ 0 };

/**
 * Get the next available port.
 */
unsigned short getavailableport( void ) {
  SpinLockGuard guard( availableportslock );

  auto ourport = availableports.front();
  availableports.pop();

  channelscreated.fetch_add( 1 );

  return ourport;
}

/**
 * Returns the number of ports we still have available.
 * @returns { size_t }
 */
auto getvailableportsize() {
  SpinLockGuard guard( availableportslock );
  return availableports.size();
}

/**
 * Return a port number to the available list (to the back) and clear our value
 * so we cannot return it twice.
 */
void projectrtpchannel::returnavailableport( void ) {

  if( 0 == this->port ) return;
  SpinLockGuard guard( availableportslock );

  availableports.push( this->port );
  this->port = 0;

  channelscreated.fetch_sub( 1 );
}

/* useful for random string generation */
const char alphanumsecret[] = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890";

/*
# RTP Channel

This file represents an RP channel. That is an RTP stream (UDP) with
its pair RTCP socket. Basic functions for

1. Opening and closing channels
2. bridging 2 channels
3. Sending data to an endpoint based on us receiving data first or (to be
implimented) the address and port given to us when opening in the channel.

*/
projectrtpchannel::projectrtpchannel( unsigned short port ):
  /* public */
  _requestclose( false ),
  closereason(),
  codec( 0 ),
  ssrcin( 0 ),
  ssrcout( 0 ),
  tsout( 0 ),
  snout( 0 ),
  send( true ),
  recv( true ),
  receivedpkcount( 0 ),
  receivedpkskip( 0 ),
  maxticktime( 0 ),
  totalticktime( 0 ),
  totaltickcount( 0 ),
  tickswithnortpcount( 0 ),
  hardtickswithnortpcount( 0 ),
  outpkwritecount( 0 ),
  outpkcount( 0 ),
  outpkskipcount( 0 ),
  outpkdropcount( 0 ),
  inbuff( rtpbuffer::create() ),
  rtpbufferlock( false ),
  rtpoutindex( 0 ),
  icelocalpwd(),
  iceremotepwd(),
#ifdef NODE_MODULE
  cb( NULL ),
#endif
  /* private */
  active( false ),
  port( port ),
  rfc2833pt( RFC2833PAYLOADTYPE ),
  ilbcpt( ILBCPAYLOADTYPE ),
  lasttelephoneeventsn( 0 ),
  lasttelephoneevent( 0 ),
  resolver( workercontext ),
  rtpsocket( workercontext ),
  rtcpsocket( workercontext ),
  rtpsenderendpoint(),
  confirmedrtpsenderendpoint(),
  rtcpsenderendpoint(),
  receivedrtp( false ),
  remoteconfirmed( false ),
  autoadjust( true ),
  mixerlock( false ),
  mixer( nullptr ),
  mixing( false ),
  removemixer( false ),
  outcodec(),
  incodec(),
  player( nullptr ),
  playerstash( nullptr ),
  playerlock( false ),
  doecho( false ),
  tick( workercontext ),
  nexttick( std::chrono::high_resolution_clock::now() ),
  recorderslock( false ),
  recorders(),
  rtpdtls( nullptr ),
  rtpdtlslock( false ),
  dtlsmode( dtlssession::none ),
  dtlsfingerprint( "" ),
  remoteaddress(),
  remoteport( 0 ),
  queueddigits(),
  queuddigitslock( false ),
  lastdtmfsn( 0 ),
  tickstarttime() {

  gnutls_rnd( GNUTLS_RND_RANDOM, &this->ssrcout, 4 );

  char localicepwd[ 25 ];
  localicepwd[ 0 ] = 0;
  if( 0 == gnutls_rnd( GNUTLS_RND_RANDOM, localicepwd, sizeof( localicepwd ) ) ) {
    for( size_t i = 0; i < sizeof( localicepwd ) - 1; i++ ) {
      localicepwd[ i ] = alphanumsecret[ localicepwd[ i ] % ( sizeof( alphanumsecret ) - 1 ) ];
    }
  }
  localicepwd[ 24 ] = 0;

  this->icelocalpwd = localicepwd;
}

void projectrtpchannel::requestclose( std::string reason ) {

  this->closereason = reason;
  if( this->mixing ) {
    this->removemixer = true;
  } else {
    this->_requestclose.exchange( true, std::memory_order_acquire );
  }
}

void projectrtpchannel::remote( std::string address,
                                unsigned short port,
                                uint32_t codec,
                                unsigned short ilbcpt,
                                unsigned short rfc2833pt,
                                dtlssession::mode m,
                                std::string fingerprint ) {

  /* track changes which invalidate dtls session */
  bool changed = false;
  if( "" == address ) return;

  if( address != this->remoteaddress ) {
    this->remoteaddress = address;
    changed = true;
  }
  
  if( port != this->remoteport ) {
    this->remoteport = port;
    changed = true;
  }

  
  if( this->dtlsfingerprint != fingerprint ) {
    this->dtlsfingerprint = fingerprint;
    changed = true;
  }

  if( this->dtlsmode != m ) {
    this->dtlsmode = m;
    changed = true;
  }

  this->codec = codec;
  this->rfc2833pt = rfc2833pt;
  this->ilbcpt = ilbcpt;

  if( changed ) {
    if( dtlssession::none != m ) {
      dtlssession::pointer newsession = dtlssession::create( m );
      newsession->setpeersha256( fingerprint );

      projectrtpchannel::pointer p = shared_from_this();
      newsession->ondata( [ p ] ( const void *d , size_t l ) -> void {
        /* Note to me, I need to confirm that gnutls maintains the buffer ptr until after the handshake is confirmed (or
          at least until we have sent the packet). */
        if( p->remoteconfirmed ) {
          p->rtpsocket.async_send_to(
                            boost::asio::buffer( d, l ),
                            p->confirmedrtpsenderendpoint,
                            []( const boost::system::error_code& ec, std::size_t bytes_transferred ) -> void {
                              /* We don't need to do anything */
                            } );
        }
      } );

      {
        SpinLockGuard guard( this->rtpdtlslock );
        this->rtpdtls = newsession;
      }
    }
    
    this->receivedrtp = false;
    this->remoteconfirmed = false;
    this->autoadjust = true;

    this->doremote();
  }
}

/**
 * Convert our requested address and port into boost::asio::ip::udp::endpoint
 */
void projectrtpchannel::doremote( void ) {

  if( "" == this->remoteaddress ) return;

  boost::asio::ip::udp::resolver::query query(
    boost::asio::ip::udp::v4(),
    this->remoteaddress,
    std::to_string( this->remoteport ) );

  /* Resolve the address */
  this->resolver.async_resolve( query,
      boost::bind( &projectrtpchannel::handleremoteresolve,
        shared_from_this(),
        boost::asio::placeholders::error,
        boost::asio::placeholders::iterator ) );
}

/**
 * handleremoteresolve
 * We have resolved the remote address and port now use it. Further work could be to inform control there is an issue.
 */
void projectrtpchannel::handleremoteresolve (
            boost::system::error_code e,
            boost::asio::ip::udp::resolver::iterator it ) {
  boost::asio::ip::udp::resolver::iterator end;

  if( e == boost::asio::error::operation_aborted ) return;

  if( it == end ) {
    /* Failure - silent (the call will be as well!) */
    this->requestclose( "failed.remote" );
    return;
  }

  this->confirmedrtpsenderendpoint = *it;
  this->remoteconfirmed = true;
  this->hardtickswithnortpcount = 0;

  /* allow us to re-auto correct */
  this->autoadjust = true;

  /* allow us to grab ssrc */
  this->receivedrtp = false;
}


/**
 * requestopen - post a request to call doopen in the worker context.
 */
uint32_t projectrtpchannel::requestopen( void ) {

  /* this shouldn't happen */
  if( this->active ) return this->ssrcout;

  boost::asio::post( workercontext,
        boost::bind( &projectrtpchannel::doopen, shared_from_this() ) );

  return this->ssrcout;
}

/**
 * handle errors on socket open
*/
void projectrtpchannel::badsocketopen( const char *err ) {

  fprintf( stderr, "Socket issue - refusing a new channel: %s\n", err );

  if( this->rtpsocket.is_open()  ) {
    this->rtpsocket.close();
  }

  if( !this->rtcpsocket.is_open() ) {
    this->rtcpsocket.close();
  }

  this->returnavailableport();

  postdatabacktojsfromthread( shared_from_this(), "close", "error.nosocket" );
}

/**
 * Must be called in the workercontext.
 */
void projectrtpchannel::doopen( void ) {

  boost::system::error_code ec;

  this->outcodec.reset();
  this->incodec.reset();

  this->rtpsocket.open( boost::asio::ip::udp::v4(), ec );

  if( ec ) {
    this->badsocketopen( "failed to open rtp socket" );
    return;
  }

  this->rtpsocket.bind( boost::asio::ip::udp::endpoint(
      boost::asio::ip::udp::v4(), this->port ), ec );

  if( ec ) {
    auto err = std::string( "failed to bind rtp socket: " ) + std::to_string( this->port ) + std::string( ": " ) + ec.message();
    this->badsocketopen( err.c_str() );
    return;
  }

  this->rtcpsocket.open( boost::asio::ip::udp::v4(), ec );

  if( ec ) {
    this->badsocketopen( "failed to open rtcp socket" );
    return;
  }

  this->rtcpsocket.bind( boost::asio::ip::udp::endpoint(
      boost::asio::ip::udp::v4(), this->port + 1 ), ec );

  if( ec ) {
    auto err = std::string( "failed to bind rtcp socket: " ) + std::to_string( this->port + 1 ) + std::string( ": " ) + ec.message();
    this->badsocketopen( err.c_str() );
    return;
  }

  this->active = true;

  /* anchor our out time to when the channel is opened */
  this->tsout = std::chrono::system_clock::to_time_t( std::chrono::system_clock::now() );
  this->snout = rand();

  if( 0 != this->remoteport ) this->doremote();

  this->readsomertp();
  this->readsomertcp();

  this->nexttick = std::chrono::high_resolution_clock::now() + std::chrono::milliseconds( 20 );

  this->tick.expires_after( this->nexttick - std::chrono::high_resolution_clock::now() );
  this->tick.async_wait( boost::bind( &projectrtpchannel::handletick,
                                        shared_from_this(),
                                        boost::asio::placeholders::error ) );

}

/*
## projectrtpchannel destructor
Clean up
*/
projectrtpchannel::~projectrtpchannel( void ) {
}

/*
# create

*/
projectrtpchannel::pointer projectrtpchannel::create( unsigned short port ) {
  return pointer( new projectrtpchannel( port ) );
}

unsigned short projectrtpchannel::getport( void ) {
  return this->port;
}

void projectrtpchannel::requestecho( bool e ) {
  this->doecho = e;
}

/**
 * Worker thread to perform teh close. It will only run if the channel has
 * been active (i.e. the sockets have been opened sucsessfully)
 */
void projectrtpchannel::doclose( void ) {

  if( !this->active ) return;
  this->active = false;

  auto self = shared_from_this();

  this->tick.cancel();

  {
    SpinLockGuard guard( this->playerlock );
    if( nullptr != this->player ) {
      postdatabacktojsfromthread( self, "play", "end", "channelclosed" );
    }

    this->player = nullptr;
  }

  /* close our session if we have one */
  {
    SpinLockGuard dtlsguard( this->rtpdtlslock );
    this->rtpdtls = nullptr;
  }

  /* close up any remaining recorders */
  {
    SpinLockGuard guard( this->recorderslock );
    for( auto& rec: this->recorders ) {
      postdatabacktojsfromthread( self, "record", rec->file, "finished.channelclosed" );
    }

    this->recorders.clear();
  }

  this->resolver.cancel();

  this->rtpsocket.close();
  this->rtcpsocket.close();

  this->returnavailableport();

  postdatabacktojsfromthread( self, "close", this->closereason );
}

bool projectrtpchannel::checkidlerecv( void ) {
  if( this->recv && this->active ) {

    this->hardtickswithnortpcount++;
    if( this->remoteconfirmed ) {

      this->tickswithnortpcount++;
      if( this->tickswithnortpcount > ( 50 * 20 ) ) { /* 50 (@20mS ptime)/S = 20S */
        this->closereason = "idle";
        this->doclose();
        return true;
        }
    
    } else if( this->hardtickswithnortpcount > ( 50 * 60 * 60 ) ) { /* 1hr hard timeout */
      this->closereason = "idle";
      this->doclose();
      return true;
    }
  } else if( this->active ) {
    /* active but not receiving ie on hold or similar - but there are limits! */
    if( this->hardtickswithnortpcount > ( 50 * 60 * 60 * 2 ) ) { /* 2hr hard timeout */
      this->closereason = "idle";
      this->doclose();
      return true;
    }
  }
  return false;
}

void projectrtpchannel::incrtsout( void ) {
  this->tsout += G711PAYLOADBYTES;
}

/*!md
## handletick
Our timer to send data - use this for when we are a single channel. Mixing tick is done in mux.
*/
void projectrtpchannel::handletick( const boost::system::error_code& error ) {
  if( error == boost::asio::error::operation_aborted ) return;
  if( !this->active ) return;

  auto self = shared_from_this();

  if( this->_requestclose ) {
    this->doclose();
    return;
  }

  if( this->dtlsnegotiate() ) {
    this->setnexttick();
    return;
  }

  if( this->mixing ) {
    this->setnexttick();
    return;
  }

  this->startticktimer();

  this->incrtsout();

  if( this->checkidlerecv() ) {
    this->endticktimer();
    return;
  }

  this->incodec << codecx::next;
  this->outcodec << codecx::next;

  soundsoup::pointer ourplayer = nullptr;
  {
    SpinLockGuard guard( this->playerlock );
    ourplayer = this->player;
  }

  rtppacket *src;
  do {
    {
      SpinLockGuard guard( this->rtpbufferlock );
      src = this->inbuff->pop();
    }

    dtlssession::pointer currentdtlssession;
    {
      SpinLockGuard guarddtls( this->rtpdtlslock );
      currentdtlssession = this->rtpdtls;
    }

    if( nullptr != currentdtlssession &&
        !currentdtlssession->rtpdtlshandshakeing ) {
      if( !currentdtlssession->unprotect( src ) ) {
        this->receivedpkskip++;
        src = nullptr;
      }
    }
  } while( this->checkfordtmf( src ) );

  this->senddtmf();

  if( nullptr != src ) {
    this->incodec << *src;
  }

  if( this->doecho ) {
    if( ourplayer ) {
      postdatabacktojsfromthread( self, "play", "end", "doecho" );
      SpinLockGuard guard( this->playerlock );
      this->player = nullptr;
    }

    if( nullptr != src ) {
      this->outcodec << *src;

      rtppacket *dst = this->gettempoutbuf();
      dst << this->outcodec;
      this->writepacket( dst );
    }
  } else if( ourplayer ) {
    rtppacket *out = this->gettempoutbuf();
    rawsound r;
    if( ourplayer->read( r ) ) {
      if( r.size() > 0 ) {
        this->outcodec << r;
        out << this->outcodec;
        this->writepacket( out );
      }
    } else {
      postdatabacktojsfromthread( self, "play", "end", "completed" );
      SpinLockGuard guard( this->playerlock );
      this->player = nullptr;
    }
  }

  this->writerecordings();

  this->endticktimer();

  /* The last thing we do */
  this->setnexttick();
}

void projectrtpchannel::startticktimer( void ) {
  this->tickstarttime = boost::posix_time::microsec_clock::local_time();
}

void projectrtpchannel::endticktimer( void ) {
  boost::posix_time::time_duration const diff = ( boost::posix_time::microsec_clock::local_time() - this->tickstarttime );
  uint64_t tms = diff.total_microseconds();
  this->totalticktime += tms;
  this->totaltickcount++;
  if( tms > this->maxticktime ) this->maxticktime = tms;
}

void projectrtpchannel::setnexttick( void ) {
  this->nexttick = this->nexttick + std::chrono::milliseconds( 20 );

  this->tick.expires_after( this->nexttick - std::chrono::high_resolution_clock::now() );
  this->tick.async_wait( boost::bind( &projectrtpchannel::handletick,
                                      shared_from_this(),
                                      boost::asio::placeholders::error ) );
}

void projectrtpchannel::requestplay( soundsoup::pointer newdef ) {
  SpinLockGuard guard( this->playerlock );

  auto self = shared_from_this();

  this->doecho = false;

  if( nullptr != this->player ) {
    postdatabacktojsfromthread( self, "play", "end", "replaced" );
  }

  postdatabacktojsfromthread( self, "play", "start", "new" );
  this->player = newdef;
}

/*
Post a request to record (or modify a param of a record). This function is typically
called from a control thread (i.e. node).
*/
void projectrtpchannel::requestrecord( channelrecorder::pointer newrec ) {
  SpinLockGuard guard( this->recorderslock );

  for( auto& currentrec: this->recorders ) {
    if( currentrec->file == newrec->file ) {
      currentrec->pause.store( newrec->pause );
      currentrec->requestfinish.store( newrec->requestfinish );
      return;
    }
  }

  newrec->sfile = soundfilewriter::create(
      newrec->file,
      newrec->numchannels,
      soundfile::getsampleratefrompt( this->codec ) );


  this->recorders.push_back( newrec );
}

void projectrtpchannel::removeoldrecorders( pointer self ) {

  SpinLockGuard guard( this->recorderslock );

  for ( auto const& rec : this->recorders ) {
    if( rec->isactive() ) {

      boost::posix_time::ptime nowtime( boost::posix_time::microsec_clock::local_time() );
      boost::posix_time::time_duration const diff = ( nowtime - rec->activeat );

      if( diff.total_milliseconds() < rec->minduration  ) {
        continue;
      }

      if( rec->requestfinish ) {
        rec->completed = true;
        rec->maxsincestartpower = 0;
        postdatabacktojsfromthread( self, "record", rec->file, "finished.requested" );
        continue;
      }

      if( rec->lastpowercalc > rec->maxsincestartpower ) {
        rec->maxsincestartpower = rec->lastpowercalc;
      }

      if( rec->finishbelowpower > 0 &&
          rec->maxsincestartpower > rec->finishbelowpower &&
          rec->lastpowercalc < rec->finishbelowpower ) {
        rec->completed = true;
        rec->maxsincestartpower = 0;
        postdatabacktojsfromthread( self, "record", rec->file, "finished.belowpower" );
        continue;
      }

      if( 0 != rec->maxduration && diff.total_milliseconds() > rec->maxduration ) {
        rec->completed = true;
        rec->maxsincestartpower = 0;
        postdatabacktojsfromthread( self, "record", rec->file, "finished.timeout" );
        continue;
      }
    }
  }

  this->recorders.remove_if( recordercompleted );
}

bool projectrtpchannel::recordercompleted( const channelrecorder::pointer& rec ) {
  return rec->completed;
}

/**
 * @brief Helper function for checkfordtmf - signal back to our control server that an event has been received.
 * 
 */
static char dtmfchars[] = { '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '*', '#', 'A', 'B', 'C', 'D', 'F' };
void projectrtpchannel::sendtelevent( void ) {
  auto self = shared_from_this();
  SpinLockGuard guard( this->playerlock );

  if( this->player && this->player->doesinterupt() ) {
    postdatabacktojsfromthread( self, "play", "end", "telephone-event" );
    this->player = nullptr;
  }

  postdatabacktojsfromthread( self, "telephone-event", std::string( 1, dtmfchars[ this->lasttelephoneevent ] ) );
}
/*
## checkfordtmf

We should receive a start packet with mark set to true. This should then continue until a packet with an
end of event marked in the 2833 payload. But we might lose any one of these packets and should still work
if we do.
*/
bool projectrtpchannel::checkfordtmf( rtppacket *src ) {
  /* The next section is sending to our recipient(s) */

  if( nullptr == src ) return false;

  uint16_t sn = src->getsequencenumber();

  if( 0 != this->rfc2833pt &&
      src->getpayloadtype() == RFC2833PAYLOADTYPE ) {

    if( src->getpayloadlength() >= 4 ) {
      /* We have to look for DTMF events handling issues like missing events - such as the marker or end bit */

      uint8_t * pl = src->getpayload();
      uint8_t endbit = pl[ 1 ] >> 7;
      uint8_t event = pl[ 0 ] & 0x7f;

      if( event <= 16 ) {
        /* Test for start */
        if( 0 == endbit ) {
          this->lasttelephoneeventsn = sn;
          this->lasttelephoneevent = event;
        } else if( 1 == endbit &&
                   0 != this->lasttelephoneeventsn ) {
          this->lasttelephoneeventsn = 0;
          this->sendtelevent();
        }
      }
    }
    return true;
  } else if( 0 != this->lasttelephoneeventsn &&
             abs( static_cast< long long int > ( sn - this->lasttelephoneeventsn ) ) > MAXDTMFSNDIFFERENCE ) {
    /* timeout on waiting for end packet */
    this->sendtelevent();
    this->lasttelephoneeventsn = 0;
  }
  return false;
}


/*
# writerecordings
If our codecs (in and out) have data then write to recorded files.
*/
void projectrtpchannel::writerecordings( void ) {

  if( !this->active ) return;

  auto self = shared_from_this();

  chanrecptrlist worklist;
  {
    SpinLockGuard guard( this->recorderslock );
    worklist = this->recorders;
  }

  if( 0 == worklist.size() ) return;

  uint16_t power = 0;

  /* Decide if we need to calc power as it is expensive */
  for( auto& rec: worklist ) {
    if( ( !rec->isactive() && 0 != rec->startabovepower ) || 0 != rec->finishbelowpower ) {
      power = this->incodec.power();
      break;
    }
  }

  /* Check if we need to trigger the start of any recordings and write */
  for( auto& rec: worklist ) {

    if( rec->completed ) continue;

    /* calculate power for the below tests and the finish test in ::removeoldrecorders */
    uint16_t pav = 0;
    if( ( !rec->isactive() && rec->startabovepower > 0 ) || 
        ( rec->isactive() && rec->finishbelowpower > 0 ) ) {
      pav = rec->poweravg( power );
    }


    if( !rec->isactive() ) {
      if( 0 == rec->startabovepower ) {
        rec->active();
        postdatabacktojsfromthread( self, "record", rec->file, "recording" );
      } else {
        if( pav > rec->startabovepower ) {
          rec->active();
          postdatabacktojsfromthread( self, "record", rec->file, "recording.abovepower" );
        }
      }
    }

    if( rec->isactive() && !rec->pause ) {
      rec->sfile->write( this->incodec, this->outcodec );
    }
  }

  /* this function is also protected by this->recorderslock - so ensure it is free before calling */
  /* pass in sefl to ensure no gap in destruction/auto ptr destruction */
  this->removeoldrecorders( self );
}

bool projectrtpchannel::dtlsnegotiate( void ) {
  SpinLockGuard guard( this->rtpdtlslock );

  dtlssession::pointer oursession = this->rtpdtls;

  if( nullptr == oursession ) return false;
  if( !oursession->rtpdtlshandshakeing ) return false;

  auto dtlsstate = oursession->handshake();

  if( GNUTLS_E_SUCCESS != dtlsstate && 0 != gnutls_error_is_fatal( dtlsstate ) ) {
    oursession->bye();

    std::stringstream out;
    out << "error.dtlsfail: " << dtlsstate;
    this->requestclose( out.str() );
  }

  if( !oursession->rtpdtlshandshakeing ) {
    this->correctaddress();
  }

  return oursession->rtpdtlshandshakeing;
}

/**
 * @brief 
 * 
 * @return true 
 * @return false 
 */
bool projectrtpchannel::handlestun( uint8_t *pk, size_t len ) {

  if( stun::is( pk, len ) ) {

    size_t len = stun::handle( pk, this->stuntmpout, sizeof( this->stuntmpout ), this->rtpsenderendpoint, this->icelocalpwd, this->iceremotepwd );
    if( len > 0 ) {
      this->rtpsocket.async_send_to(
                      boost::asio::buffer( this->stuntmpout, len ),
                      this->rtpsenderendpoint, /* do we sometime tie this in with confirmedrtpsenderendpoint as the integrity check can auth it? */
                      boost::bind( &projectrtpchannel::handlesend,
                                    shared_from_this(),
                                    boost::asio::placeholders::error,
                                    boost::asio::placeholders::bytes_transferred ) );

      this->correctaddress();
    }

    return true;
  }

  return false;
}

/**
 * Auto correct our to send to address if required. 
 */
void projectrtpchannel::correctaddress( void ) {

  if( this->autoadjust ) {
    this->confirmedrtpsenderendpoint = this->rtpsenderendpoint;
    this->remoteconfirmed = true;
    this->hardtickswithnortpcount = 0;
    this->autoadjust = false;
  }
}

/**
 * Auto correct our SSRC if required
 */
void projectrtpchannel::correctssrc( uint32_t ssrc ) {
  if( !this->receivedrtp ) {
    /* allow settling */
    if( this->inbuff->getpushed() > 5 )this->receivedrtp = true;
    this->ssrcin = ssrc;
  }
}

/*
## handlereadsomertp
Wait for RTP data. We have to re-order when required. Look after all of the round robin memory here.
We should have enough time to deal with the in data before it gets overwritten.

WARNING WARNING
Order matters. We have multiple threads reading and writing variables using atomics and normal data.
Any clashes should be handled by atomics.

I have switched to using blocking functions - but only in responce to an async wait
which should mean (not very well documented) that the read will not block.
*/
void projectrtpchannel::readsomertp( void ) {

  if( !this->active || this->_requestclose ) return;

  /* Grab a buffer */
  rtppacket* buf;
  {
    SpinLockGuard guard( this->rtpbufferlock );
    buf = this->inbuff->reserve();
  }
  

  if( nullptr == buf ) {
    fprintf( stderr, "Error no buffer\n" );
    this->requestclose( "error.nobuffer" );
    return;
  }

  this->rtpsocket.async_receive_from(
  boost::asio::buffer( buf->pk, RTPMAXLENGTH  ), this->rtpsenderendpoint,
    [ this, buf ]( boost::system::error_code ec, std::size_t bytesrecvd ) {

      if ( ec && ec != boost::asio::error::message_size ) return;
      this->receivedpkcount++;

      if( !this->active || this->_requestclose ) return;

      if ( bytesrecvd > 0 && bytesrecvd <= RTPMAXLENGTH ) {
        buf->length = bytesrecvd;

        if( this->handlestun( buf->pk, bytesrecvd ) )
          goto readsomemore;

        dtlssession::pointer currentdtlssession;
        {
          SpinLockGuard guard( this->rtpdtlslock );
          currentdtlssession = this->rtpdtls;
        }

        if( nullptr != currentdtlssession &&
            currentdtlssession->rtpdtlshandshakeing ) {
          {
            SpinLockGuard guard( this->rtpdtlslock );
            currentdtlssession->write( buf->pk, bytesrecvd );
          }

          this->dtlsnegotiate();
          goto readsomemore;
        }

        /* TODO ZRTP? */
        if( buf->getpacketextension() ) {
          this->receivedpkskip++;
          goto readsomemore;
        }

        this->correctaddress();

        if( !this->recv ) {
          this->receivedpkskip++;
          goto readsomemore;
        }

        /*
        after speaking with Magrathea - more streams are changing ssrc without notice mid-stream
        for now do not check.
        this->correctssrc( buf->getssrc() );
        */
        
        if( this->confirmedrtpsenderendpoint != this->rtpsenderendpoint ) {
          /* After the first packet - we only accept data from the verified source */
          this->receivedpkskip++;
          goto readsomemore;
        }

        /*
        if( buf->getssrc() != this->ssrcin ) {
          this->receivedpkskip++;
          goto readsomemore;
        }
        */

        this->tickswithnortpcount = 0;
        this->hardtickswithnortpcount = 0;

        /* dynamic payload types */
        auto pt = buf->getpayloadtype();
        if( pt == this->ilbcpt ) {
          buf->setpayloadtype( ILBCPAYLOADTYPE );
        } else if ( pt == this->rfc2833pt ) {
          buf->setpayloadtype( RFC2833PAYLOADTYPE );
        }

        {
          SpinLockGuard guard( this->rtpbufferlock );
          this->inbuff->push();
        }
      }

readsomemore:
      this->readsomertp();

    } );
}

/*
## gettempoutbuf
When we need a buffer to send data out (because we cannot guarantee our
own buffer will be available) we can use the circular out buffer on this
channel. This will return the next one available.

We assume this is called to send packets out in order, and at intervals
 required for each timestamp to be incremented in lou of it payload type.
*/
rtppacket *projectrtpchannel::gettempoutbuf( void ) {

  uint16_t outindex = this->rtpoutindex++;
  rtppacket *buf = &this->outrtpdata[ ( outindex % OUTBUFFERPACKETCOUNT ) ];

  buf->init( this->ssrcout );
  buf->setpayloadtype( this->codec );
  buf->setsequencenumber( this->snout );
  buf->settimestamp( this->tsout );
  return buf;
}

/*
## handlereadsomertcp
Wait for RTP data
*/
void projectrtpchannel::readsomertcp( void ) {
  this->rtcpsocket.async_receive_from(
  boost::asio::buffer( &this->rtcpdata[ 0 ], RTCPMAXLENGTH ), this->rtcpsenderendpoint,
    [ this ]( boost::system::error_code ec, std::size_t bytes_recvd ) {

      if( ec ) return;

      if ( bytes_recvd > 0 && bytes_recvd <= RTCPMAXLENGTH ) {
        this->handlertcpdata();
      }

      if( bytes_recvd > 0 && this->active ) {
        this->readsomertcp();
      }
    } );
}

/*
## isactive
As it says.
*/
bool projectrtpchannel::isactive( void ) {
  return this->active;
}

/*
## writepacket
Send a [RTP] packet to our endpoint.
*/
void projectrtpchannel::writepacket( rtppacket *pk ) {

  if( !this->active ) return;

  dtlssession::pointer currentdtlssession;
  {
    SpinLockGuard guard( this->rtpdtlslock );
    currentdtlssession = this->rtpdtls;
  }

  if( nullptr != currentdtlssession &&
      currentdtlssession->rtpdtlshandshakeing ) {
    goto completewrite;
  }

  if( nullptr == pk ) {
    fprintf( stderr, "We have been given an nullptr RTP packet??\n" );
    goto completewrite;
  }

  if( 0 == pk->length ) {
    fprintf( stderr, "We have been given an RTP packet of zero length??\n" );
    goto completewrite;
  }

  /* silently drop - could we do this sooner to use less CPU? */
  if( !this->send ) goto completewrite;

  if( nullptr != currentdtlssession &&
      !currentdtlssession->rtpdtlshandshakeing ) {
    if( !currentdtlssession->protect( pk ) ) {
      goto completewrite;
    }
  }

  if( this->remoteconfirmed ) {
    this->snout++;
    this->outpkwritecount++;

    /* dynamic payload types */
    auto pt = pk->getpayloadtype();
    if( pt == ILBCPAYLOADTYPE ) {
      pk->setpayloadtype( this->ilbcpt );
    } else if ( pt == RFC2833PAYLOADTYPE ) {
      pk->setpayloadtype( this->rfc2833pt );
    }

    this->rtpsocket.async_send_to(
                      boost::asio::buffer( pk->pk, pk->length ),
                      this->confirmedrtpsenderendpoint,
                      boost::bind( &projectrtpchannel::handlesend,
                                    shared_from_this(),
                                    boost::asio::placeholders::error,
                                    boost::asio::placeholders::bytes_transferred ) );
    return;
  }
completewrite:
  this->outpkskipcount++;
}

/*
## mix
Add the other to a mixer - both channels have access to the same mixer.
n way relationship. Adds to queue for when our main thread calls into us.
*/
bool projectrtpchannel::mix( projectrtpchannel::pointer other ) {
  if( this == other.get() ) {
    return true;
  }

  auto self = shared_from_this();

  {
    SpinLockGuard guard( this->playerlock );
    if( nullptr != this->player ) {
      postdatabacktojsfromthread( self, "play", "end", "channelmixing" );
    }
    this->player = nullptr;
  }
  {
    SpinLockGuard guard( other->playerlock );
    if( nullptr != other->player ) {
      postdatabacktojsfromthread( other, "play", "end", "channelmixing" );
    }
    other->player = nullptr;
  }

  {
    SpinLockGuard guard( this->mixerlock );

    if( nullptr == this->mixer && nullptr != other->mixer ) {
      this->mixer = other->mixer;
      this->mixer->addchannel( self );

    } else if ( nullptr != this->mixer && nullptr == other->mixer ) {
      other->mixer = this->mixer;
      this->mixer->addchannel( other );

    } else if( nullptr == this->mixer && nullptr == other->mixer  ) {
      this->mixer = projectchannelmux::create( workercontext );
      other->mixer = this->mixer;

      this->mixer->addchannels( self, other );

      this->mixer->go();
    } else {
      /* If we get here this and other are already mixing and should be cleaned up first */
      postdatabacktojsfromthread( self, "mix", "busy" );
      return false;
    }
  }

  postdatabacktojsfromthread( self, "mix", "start" );
  postdatabacktojsfromthread( other, "mix", "start" );

  return true;
}

/*
## mix
Add the other to a mixer - both channels have access to the same mixer.
n way relationship. Adds to queue for when our main thread calls into us.
*/
bool projectrtpchannel::unmix( void ) {
  SpinLockGuard guard( this->mixerlock );

  if( nullptr != this->mixer ) this->removemixer = true;
  return true;
}

/**
 * @brief Actual do set the variables ot show unmixed.
 * 
 */
void projectrtpchannel::dounmix( void ) {
  this->mixing = false;
  this->removemixer = false;

  SpinLockGuard guard( this->mixerlock );

  this->mixer = nullptr;

  postdatabacktojsfromthread( shared_from_this(), "mix", "finished" );

  if( this->closereason.length() > 0 ) {
    this->_requestclose.exchange( true, std::memory_order_acquire );
  }
}

/*
## dtmf
Queue digits to send as RFC 2833.
*/
void projectrtpchannel::dtmf( std::string digits ) {
  SpinLockGuard guard( this->queuddigitslock );
  this->queueddigits += digits;
}

/*
Now send each digit.
*/
void projectrtpchannel::senddtmf( void ) {

  if( static_cast< uint16_t >( this->snout - this->lastdtmfsn ) < 10 ) {
    return;
  }

  uint8_t tosend = 0;
  {
    SpinLockGuard guard( this->queuddigitslock );
    if( this->queueddigits.size() > 0 ) {
      tosend = this->queueddigits[ 0 ];
      this->queueddigits.erase( this->queueddigits.begin() );
    }
  }

  if( 0 == tosend ) {
    return;
  }

  /*
    RFC 2833:
     _________________________
     0--9                0--9
     *                     10
     #                     11
     A--D              12--15
     Flash                 16
  */
  if( tosend >= 48 && tosend <= 57 ) {
    // 0 -> 9
    tosend -= 48;
  } else if ( '*' == tosend ) {
    tosend = 10;
  } else if ( '#' == tosend ) {
    tosend = 11;
  } else if ( tosend >= 65 && tosend <= 68 ) {
    tosend = tosend + 12 - 65;
  } else if ( tosend >= 97 && tosend <= 100 ) {
    tosend = tosend + 12 - 97;
  } else if ( 'F' == tosend || 'f' == tosend ) { /* Flash */
    tosend = 16;
  } else {
    return;
  }

  const char volume = 10;
  const char endofevent = 0x80;

  rtppacket *dst = this->gettempoutbuf();
  dst->setpayloadtype( this->rfc2833pt );
  dst->setpayloadlength( 4 );
  uint8_t *pl =  dst->getpayload();
  pl[ 0 ] = tosend;
  pl[ 1 ] = volume; /* end of event & reserved & volume */
  uint16_t *tmp = ( uint16_t * ) &pl[ 2 ]; /* event duration */
  *tmp = htons( 160 );

  this->writepacket( dst );

  dst = this->gettempoutbuf();
  dst->setpayloadtype( this->rfc2833pt );
  dst->setpayloadlength( 4 );
  pl =  dst->getpayload();
  pl[ 0 ] = tosend;
  pl[ 1 ] = volume; /* end of event & reserved & volume */
  tmp = ( uint16_t * ) &pl[ 2 ]; /* event duration */
  *tmp = htons( 320 );

  this->writepacket( dst );

  dst = this->gettempoutbuf();
  dst->setpayloadtype( this->rfc2833pt );
  dst->setpayloadlength( 4 );
  pl =  dst->getpayload();
  pl[ 0 ] = tosend;
  pl[ 1 ] = volume; /* end of event & reserved & volume */
  tmp = ( uint16_t * ) &pl[ 2 ]; /* event duration */
  *tmp = htons( 480 );

  this->writepacket( dst );

  dst = this->gettempoutbuf();
  dst->setpayloadtype( this->rfc2833pt );
  dst->setmarker();
  dst->setpayloadlength( 4 );
  pl =  dst->getpayload();
  pl[ 0 ] = tosend;
  pl[ 1 ] = endofevent | volume; /* end of event & reserved & volume */
  tmp = ( uint16_t * ) &pl[ 2 ]; /* event duration */
  *tmp = htons( 640 );

  this->writepacket( dst );

  dst = this->gettempoutbuf();
  dst->setpayloadtype( this->rfc2833pt );
  dst->setpayloadlength( 4 );
  pl =  dst->getpayload();
  pl[ 0 ] = tosend;
  pl[ 1 ] = endofevent | volume; /* end of event & reserved & volume */
  tmp = ( uint16_t * ) &pl[ 2 ]; /* event duration */
  *tmp = htons( 640 );

  this->writepacket( dst );

  dst = this->gettempoutbuf();
  dst->setpayloadtype( this->rfc2833pt );
  dst->setpayloadlength( 4 );
  pl =  dst->getpayload();
  pl[ 0 ] = tosend;
  pl[ 1 ] = endofevent | volume; /* end of event & reserved & volume */
  tmp = ( uint16_t * ) &pl[ 2 ]; /* event duration */
  *tmp = htons( 640 );

  this->writepacket( dst );

  this->lastdtmfsn = this->snout;
}

/*
## handlesend
What is called once we have sent something.
*/
void projectrtpchannel::handlesend(
      const boost::system::error_code& error,
      std::size_t bytes_transferred) {

  if( !error && bytes_transferred > 0 ) {
    this->outpkcount++;
  } else {
    fprintf( stderr, "Problem sending packet\n" );
    this->outpkdropcount++;
  }
}

/*!md
## handlertcpdata
We have received some RTCP data - now do something with it.
*/
void projectrtpchannel::handlertcpdata( void ) {

}


#ifdef NODE_MODULE

// uuidgen | sed -r -e 's/-//g' -e 's/(.{16})(.*)/0x\1, 0x\2/'
static const napi_type_tag channelcreatetag = {
  0x7616402bbbee4a21, 0x81dbc4cc8f07ebc3
};

static napi_value createnapibool( napi_env env, bool v ) {
  napi_value result;
  napi_create_uint32( env, v == true? 1 : 0, &result );
  napi_coerce_to_bool( env, result, &result );
  return result;
}

static projectrtpchannel::pointer getrtpchannelfromthis( napi_env env, napi_callback_info info ) {
  size_t argc = 1;
  napi_value argv[ 1 ];
  napi_value thisarg;
  bool isrtpchannel;

  hiddensharedptr *pb;

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, &thisarg, nullptr ) ) return nullptr;
  if( napi_ok != napi_check_object_type_tag( env, thisarg, &channelcreatetag, &isrtpchannel ) ) return nullptr;

  if( !isrtpchannel ) {
    napi_throw_type_error( env, "0", "Not an RTP Channel type" );
    return nullptr;
  }

  if( napi_ok != napi_unwrap( env, thisarg, ( void** ) &pb ) ) {
    napi_throw_type_error( env, "1", "Channel didn't unwrap" );
    return nullptr;
  }

  return pb->get< projectrtpchannel >();
}

static projectrtpchannel::pointer getrtpchannelfromargv( napi_env env, napi_callback_info info ) {
  size_t argc = 1;
  napi_value argv[ 1 ];
  napi_value thisarg;
  bool isrtpchannel;

  hiddensharedptr *pb;

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, &thisarg, nullptr ) ) return nullptr;
  if( napi_ok != napi_check_object_type_tag( env, argv[ 0 ], &channelcreatetag, &isrtpchannel ) ) return nullptr;

  if( !isrtpchannel ) {
    napi_throw_type_error( env, "0", "Not an RTP Channel type" );
    return nullptr;
  }

  if( napi_ok != napi_unwrap( env, argv[ 0 ], ( void** ) &pb ) ) {
    napi_throw_type_error( env, "1", "Channel didn't unwrap" );
    return nullptr;
  }

  return pb->get< projectrtpchannel >();
}

static napi_value channelclose( napi_env env, napi_callback_info info ) {

  projectrtpchannel::pointer chan = getrtpchannelfromthis( env, info );
  if( nullptr == chan ) createnapibool( env, false );
  chan->requestclose( "requested" );

  return createnapibool( env, true );
}

/*
We receive an object like:
{
   "file": "filename",
   // optional
   "startabovepower": 250,
   "finishbelowpower": 200,
   "minduration": 2000,
   "maxduration": 15000,
   "poweraveragepackets": 50,
   "numchannels": 1 // 1 or 2
}
*/
static napi_value channelrecord( napi_env env, napi_callback_info info ) {
  size_t argc = 1;
  napi_value argv[ 1 ];

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, nullptr, nullptr ) ) {
    return createnapibool( env, false );
  }

  if( 1 != argc ) {
    return createnapibool( env, false );
  }

  projectrtpchannel::pointer chan = getrtpchannelfromthis( env, info );
  if( nullptr == chan ) {
    return createnapibool( env, false );
  }

  napi_value mfile;
  if( napi_ok != napi_get_named_property( env, argv[ 0 ], "file", &mfile ) ) {
    return createnapibool( env, false );
  }

  size_t bytescopied;
  char buf[ 256 ];

  if( napi_ok != napi_get_value_string_utf8( env, mfile, buf, sizeof( buf ), &bytescopied ) ) {
    return createnapibool( env, false );
  }

  channelrecorder::pointer p = channelrecorder::create( buf );

  /* optional */
  napi_value mtmp;
  int32_t vtmp;

  bool hasit;

  if( napi_ok == napi_has_named_property( env, argv[ 0 ], "startabovepower", &hasit ) &&
      hasit &&
      napi_ok == napi_get_named_property( env, argv[ 0 ], "startabovepower", &mtmp ) ) {
    napi_get_value_int32( env, mtmp, &vtmp );
    p->startabovepower = vtmp;
  }

  napi_has_named_property( env, argv[ 0 ], "finishbelowpower", &hasit );
  if( hasit && napi_ok == napi_get_named_property( env, argv[ 0 ], "finishbelowpower", &mtmp ) ) {
    napi_get_value_int32( env, mtmp, &vtmp );
    p->finishbelowpower = vtmp;
  }

  if( napi_ok == napi_has_named_property( env, argv[ 0 ], "minduration", &hasit ) &&
      hasit &&
      napi_ok == napi_get_named_property( env, argv[ 0 ], "minduration", &mtmp ) ) {
    napi_get_value_int32( env, mtmp, &vtmp );
    p->minduration = vtmp;
  }

  if( napi_ok == napi_has_named_property( env, argv[ 0 ], "maxduration", &hasit ) &&
      hasit &&
      napi_ok == napi_get_named_property( env, argv[ 0 ], "maxduration", &mtmp ) ) {
    napi_get_value_int32( env, mtmp, &vtmp );
    p->maxduration = vtmp;
  }

  if( napi_ok == napi_has_named_property( env, argv[ 0 ], "poweraveragepackets", &hasit ) &&
      hasit &&
      napi_ok == napi_get_named_property( env, argv[ 0 ], "poweraveragepackets", &mtmp ) ) {
    napi_get_value_int32( env, mtmp, &vtmp );
    p->poweraveragepackets = vtmp;
  }

  if( napi_ok == napi_has_named_property( env, argv[ 0 ], "pause", &hasit ) &&
      hasit &&
      napi_ok == napi_get_named_property( env, argv[ 0 ], "pause", &mtmp ) ) {
    bool vpause;
    if( napi_ok == napi_get_value_bool( env, mtmp, &vpause ) ) {
      p->pause = vpause;
    }
  }

  if( napi_ok == napi_has_named_property( env, argv[ 0 ], "finish", &hasit ) &&
      hasit &&
      napi_ok == napi_get_named_property( env, argv[ 0 ], "finish", &mtmp ) ) {
    bool vfinish;
    if( napi_ok == napi_get_value_bool( env, mtmp, &vfinish ) ) {
      p->requestfinish = vfinish;
    }
  }

  if( napi_ok == napi_has_named_property( env, argv[ 0 ], "numchannels", &hasit ) &&
      hasit &&
      napi_ok == napi_get_named_property( env, argv[ 0 ], "numchannels", &mtmp ) ) {
        
    uint32_t numchannels = 2;
    napi_get_value_uint32( env, mtmp, &numchannels );
    if( 1 == numchannels || 2 == numchannels ) {
      p->numchannels = numchannels;
    }
  }

  chan->requestrecord( p );

  return createnapibool( env, true );
}

static napi_value channelplay( napi_env env, napi_callback_info info ) {
  size_t argc = 1;
  napi_value argv[ 1 ];

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, nullptr, nullptr ) ) return NULL;

  if( 1 != argc ) {
    return createnapibool( env, false );
  }

  projectrtpchannel::pointer chan = getrtpchannelfromthis( env, info );
  if( nullptr == chan ) {
    return createnapibool( env, false );
  }

  soundsoup::pointer p = soundsoupcreate( env, argv[ 0 ], chan->codec );

  if( nullptr == p ) {
    return createnapibool( env, false );
  }

  if( 0 == p->size() ) {
    return createnapibool( env, false );
  }

  chan->requestplay( p );

  return createnapibool( env, true );
}

static napi_value channelecho( napi_env env, napi_callback_info info ) {

  size_t argc = 1;
  napi_value argv[ 1 ];

  bool echo = true;

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, nullptr, nullptr ) ) return NULL;

  if( argc > 0 ) {
    if( napi_ok == napi_get_value_bool( env, argv[ 0 ], &echo ) ) {
      return createnapibool( env, false );
    }
  }

  projectrtpchannel::pointer chan = getrtpchannelfromthis( env, info );
  if( nullptr == chan ) return createnapibool( env, false );

  chan->requestecho( echo );

  return createnapibool( env, true );
}

static napi_value channelmix( napi_env env, napi_callback_info info ) {

  size_t argc = 1;
  napi_value argv[ 1 ];

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, nullptr, nullptr ) ) return NULL;

  if( 1 != argc ) {
    napi_throw_error( env, "0", "We require 1 param (channel)" );
    return NULL;
  }

  projectrtpchannel::pointer chan = getrtpchannelfromthis( env, info );
  if( nullptr == chan ) {
    napi_throw_error( env, "1", "That's embarrassing - we shouldn't get here" );
    return NULL;
  }

  projectrtpchannel::pointer chan2 = getrtpchannelfromargv( env, info );
  if( nullptr == chan2 ) {
    napi_throw_error( env, "1", "Also embarrassing - we shouldn't get here either" );
    return NULL;
  }

  return createnapibool( env, chan->mix( chan2 ) );
}

static napi_value channelunmix( napi_env env, napi_callback_info info ) {
  size_t argc = 1;
  napi_value argv[ 1 ];

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, nullptr, nullptr ) ) return NULL;

  projectrtpchannel::pointer chan = getrtpchannelfromthis( env, info );
  if( nullptr == chan ) {
    napi_throw_error( env, "1", "That's embarrassing - we shouldn't get here" );
    return NULL;
  }

  return createnapibool( env, chan->unmix() );
}

static napi_value channeldirection( napi_env env, napi_callback_info info ) {
  size_t argc = 1;
  napi_value argv[ 1 ];

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, nullptr, nullptr ) ) return NULL;

  if( 1 != argc ) {
    napi_throw_error( env, "0", "We require 1 param" );
    return NULL;
  }

  projectrtpchannel::pointer chan = getrtpchannelfromthis( env, info );
  if( nullptr == chan ) {
    napi_throw_error( env, "1", "That's embarrassing - we shouldn't get here" );
    return NULL;
  }

  bool hasit;
  napi_value nsend, nrecv;
  if( napi_ok == napi_has_named_property( env, argv[ 0 ], "send", &hasit ) &&
      hasit &&
      napi_ok == napi_get_named_property( env, argv[ 0 ], "send", &nsend ) ) {
    bool vsend;
    if( napi_ok == napi_get_value_bool( env, nsend, &vsend ) ) {
      chan->send = vsend;
    }
  }
  if( napi_ok == napi_has_named_property( env, argv[ 0 ], "recv", &hasit ) &&
      hasit &&
      napi_ok == napi_get_named_property( env, argv[ 0 ], "recv", &nrecv ) ) {
    bool vrecv;
    if( napi_ok == napi_get_value_bool( env, nrecv, &vrecv ) ) {
      chan->recv = vrecv;
    }
  }

  return createnapibool( env, true );
}

/* Can be called on a running channel so must be thread safe */
static napi_value channelremote( napi_env env, napi_callback_info info ) {

  size_t argc = 1;
  napi_value argv[ 1 ];

  napi_value nport, naddress, ncodec;
  int32_t remoteport;

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, nullptr, nullptr ) ||
      1 != argc ) {
    return createnapibool( env, false );
  }

  projectrtpchannel::pointer chan = getrtpchannelfromthis( env, info );
  if( nullptr == chan ) {
    return createnapibool( env, false );
  }

  if( napi_ok != napi_get_named_property( env, argv[ 0 ], "port", &nport ) ) {
    return createnapibool( env, false );
  }

  napi_get_value_int32( env, nport, &remoteport );

  if( napi_ok != napi_get_named_property( env, argv[ 0 ], "address", &naddress ) ) {
    return createnapibool( env, false );
  }

  if( napi_ok != napi_get_named_property( env, argv[ 0 ], "codec", &ncodec ) ) {
    return createnapibool( env, false );
  }

  uint32_t codecval = 0;
  napi_get_value_uint32( env, ncodec, &codecval );

  uint32_t ilbcpt = ILBCPAYLOADTYPE;
  uint32_t rfc2833pt = RFC2833PAYLOADTYPE;

  bool hasit;
  napi_value nptval;
  if( napi_ok == napi_has_named_property( env, argv[ 0 ], "ilbcpt", &hasit ) &&
      hasit &&
      napi_ok == napi_get_named_property( env, argv[ 0 ], "ilbcpt", &nptval ) ) {
    napi_get_value_uint32( env, nptval, &ilbcpt );
  }

  if( napi_ok == napi_has_named_property( env, argv[ 0 ], "rfc2833pt", &hasit ) &&
      hasit &&
      napi_ok == napi_get_named_property( env, argv[ 0 ], "rfc2833pt", &nptval ) ) {
    napi_get_value_uint32( env, nptval, &rfc2833pt );
  }

  size_t bytescopied;
  char remoteaddress[ 128 ];

  napi_get_value_string_utf8( env, naddress, remoteaddress, sizeof( remoteaddress ), &bytescopied );
  if( 0 == bytescopied || bytescopied >= sizeof( remoteaddress ) ) {
    return createnapibool( env, false );
  }

  /* optional - DTLS */
  napi_value dtls;
  dtlssession::mode dtlsmode = dtlssession::none;
  char vfingerprint[ 128 ];
  vfingerprint[ 0 ] = 0;

  bool dtlsrequired = false, dtlsenabled = false;

  if ( napi_ok == napi_has_named_property( env, argv[ 0 ], "dtls", &hasit ) &&
       hasit &&
       napi_ok == napi_get_named_property( env, argv[ 0 ], "dtls", &dtls ) ) {
    dtlsrequired = true;
    napi_value nfingerprint, nactpass;
    if( napi_ok == napi_has_named_property( env, dtls, "fingerprint", &hasit ) &&
        hasit &&
        napi_ok == napi_get_named_property( env, dtls, "fingerprint", &nfingerprint ) ) {
      size_t bytescopied;
      char vactpass[ 128 ];

      napi_value nhash;
      if( napi_ok != napi_has_named_property( env, nfingerprint, "hash", &hasit ) ||
          !hasit ||
          napi_ok != napi_get_named_property( env, nfingerprint, "hash", &nhash ) ) goto nodtls;

      if( napi_ok == napi_get_value_string_utf8( env, nhash, vfingerprint, sizeof( vfingerprint ), &bytescopied ) ) {
        if( 95 != bytescopied ) {
          goto nodtls;
        }
      }

      if( napi_ok == napi_has_named_property( env, dtls, "mode", &hasit ) &&
          hasit &&
          napi_ok == napi_get_named_property( env, dtls, "mode", &nactpass ) ) {
        if( napi_ok == napi_get_value_string_utf8( env, nactpass, vactpass, sizeof( vactpass ), &bytescopied ) ) {
          if( std::string( vactpass ) == "passive" ) {
            dtlsmode = dtlssession::pass;
          } else {
            dtlsmode = dtlssession::act;
          }
          dtlsenabled = true;
        }
      }

      nodtls:
      if( dtlsrequired && !dtlsenabled ) {
        napi_throw_error( env, "1", "DTLS requested but not possible" );
        return NULL;
      }
    }
  }

  chan->remote( remoteaddress, remoteport, codecval, ilbcpt, rfc2833pt, dtlsmode, vfingerprint );

  return createnapibool( env, true );
}

static napi_value channeldtmf( napi_env env, napi_callback_info info ) {

  size_t argc = 1;
  napi_value argv[ 1 ];

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, nullptr, nullptr ) ) return NULL;

  if( 1 != argc ) {
    napi_throw_error( env, "0", "We require 1 param" );
    return NULL;
  }

  projectrtpchannel::pointer chan = getrtpchannelfromthis( env, info );
  if( nullptr == chan ) {
    napi_throw_error( env, "1", "That's embarrassing - we shouldn't get here" );
    return NULL;
  }

  size_t bytescopied;
  char dtmfstr[ 50 ];
  napi_get_value_string_utf8( env, argv[ 0 ], dtmfstr, sizeof( dtmfstr ), &bytescopied );
  if( 0 == bytescopied || bytescopied >= sizeof( dtmfstr ) ) {
    napi_throw_error( env, "2", "DTMF String too long" );
    return NULL;
  }

  chan->dtmf( std::string( dtmfstr ) );

  return createnapibool( env, true );
}


void channeldestroy( napi_env env, void* /* data */, void* hint ) {
  hiddensharedptr *pb = ( hiddensharedptr * ) hint;
  delete pb;
}

void postdatabacktojsfromthread( projectrtpchannel::pointer p, std::string event, std::string arg1, std::string arg2 ) {

  if( NULL == p->cb ) return;
  /*
    The return might be ignorable? We have created a threadsafe function
    with a max_queue_size with no limit. Can this still hit a limit?
  */
  switch( napi_call_threadsafe_function( p->cb,
                                         new jschannelevent( p, event, arg1, arg2 ),
                                         napi_tsfn_nonblocking ) ) {

  case napi_queue_full:
    fprintf( stderr, "napi_call_threadsafe_function queue full - this shouldn't happen - investigate\n" );
    break;
  case napi_ok:
    break;
  default:
    break;
  }
}
/*
  Some terms.
  dropped is when the in buffer decided the packet received is no longer valid - i.e. it is outside of our receve window
  skip is when we are processing RTP we have jumped (unexpectantly) in sequence number
*/
napi_value createcloseobject( napi_env env, projectrtpchannel::pointer p ) {

  napi_value result, action, reason;
  if( napi_ok != napi_create_object( env, &result ) ) return NULL;
  if( napi_ok != napi_create_string_utf8( env, "close", NAPI_AUTO_LENGTH, &action ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "action", action ) ) return NULL;

  if( napi_ok != napi_create_string_utf8( env, p->closereason.c_str(), NAPI_AUTO_LENGTH, &reason ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "reason", reason ) ) return NULL;

  /* calculate mos - calc borrowed from FS - thankyou. */
  napi_value mos, in, out, tick;
  if( napi_ok != napi_create_object( env, &in ) ) return NULL;
  if( napi_ok != napi_create_object( env, &out ) ) return NULL;
  if( napi_ok != napi_create_object( env, &tick ) ) return NULL;

  if( p->receivedpkcount > 0 ) {
    double r = ( ( p->receivedpkcount - p->receivedpkskip ) / p->receivedpkcount ) * 100.0;
    if ( r < 0 || r > 100 ) r = 100;
    double mosval = 1 + ( 0.035 * r ) + ( .000007 * r * ( r - 60 ) * ( 100 - r ) );

    if( napi_ok != napi_create_double( env, mosval, &mos ) ) return NULL;
  } else {
    if( napi_ok != napi_create_double( env, 0.0, &mos ) ) return NULL;
  }

  napi_value receivedpkcount, receivedpkskip, receiveddropped, resceivedpushed, resceivedpopped, bufferbadsn;
  if( napi_ok != napi_create_double( env, p->receivedpkcount, &receivedpkcount ) ) return NULL;
  if( napi_ok != napi_create_double( env, p->receivedpkskip, &receivedpkskip ) ) return NULL;
  if( napi_ok != napi_create_double( env, p->inbuff->getdropped(), &receiveddropped ) ) return NULL;
  if( napi_ok != napi_create_double( env, p->inbuff->getpopped(), &resceivedpopped ) ) return NULL;
  if( napi_ok != napi_create_double( env, p->inbuff->getpushed(), &resceivedpushed ) ) return NULL;
  if( napi_ok != napi_create_double( env, p->inbuff->getbadsn(), &bufferbadsn ) ) return NULL;

  if( napi_ok != napi_set_named_property( env, in, "mos", mos ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, in, "count", receivedpkcount ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, in, "dropped", receiveddropped ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, in, "popped", resceivedpopped ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, in, "pushed", resceivedpushed ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, in, "skip", receivedpkskip ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, in, "badsn", bufferbadsn ) ) return NULL;

  napi_value sentpkcount, outpkskipcount, outdropcount, outwritecount;
  if( napi_ok != napi_create_double( env, p->outpkcount, &sentpkcount ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, out, "count", sentpkcount ) ) return NULL;

  if( napi_ok != napi_create_double( env, p->outpkskipcount, &outpkskipcount ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, out, "skip", outpkskipcount ) ) return NULL;

  if( napi_ok != napi_create_double( env, p->outpkdropcount, &outdropcount ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, out, "drop", outdropcount ) ) return NULL;

  if( napi_ok != napi_create_double( env, p->outpkwritecount, &outwritecount ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, out, "write", outwritecount ) ) return NULL;

  napi_value meanticktimeus, maxticktimeus, totaltickcount;
  if( p->totaltickcount > 0 ) {
    if( napi_ok != napi_create_double( env, ( p->totalticktime / p->totaltickcount ), &meanticktimeus ) ) return NULL;
  } else {
    if( napi_ok != napi_create_double( env, 0, &meanticktimeus ) ) return NULL;
  }

  if( napi_ok != napi_create_double( env, p->maxticktime, &maxticktimeus ) ) return NULL;
  if( napi_ok != napi_create_double( env, p->totaltickcount, &totaltickcount ) ) return NULL;

  if( napi_ok != napi_set_named_property( env, tick, "meanus", meanticktimeus ) ) return NULL;

  if( napi_ok != napi_set_named_property( env, tick, "maxus", maxticktimeus ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, tick, "count", totaltickcount ) ) return NULL;

  napi_value stats;
  if( napi_ok != napi_create_object( env, &stats ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, stats, "in", in ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, stats, "out", out ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, stats, "tick", tick ) ) return NULL;

  if( napi_ok != napi_set_named_property( env, result, "stats", stats ) ) return NULL;
  return result;
}

napi_value createrecordobject( napi_env env, projectrtpchannel::pointer p, jschannelevent *ev ) {

  napi_value result, tmp;
  if( napi_ok != napi_create_object( env, &result ) ) return NULL;

  if( napi_ok != napi_create_string_utf8( env, "record", NAPI_AUTO_LENGTH, &tmp ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "action", tmp ) ) return NULL;

  if( napi_ok != napi_create_string_utf8( env, ev->arg1.c_str(), NAPI_AUTO_LENGTH, &tmp ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "file", tmp ) ) return NULL;

  if( napi_ok != napi_create_string_utf8( env, ev->arg2.c_str(), NAPI_AUTO_LENGTH, &tmp ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "event", tmp ) ) return NULL;

  return result;
}

napi_value createmixobject( napi_env env, projectrtpchannel::pointer p, jschannelevent *ev ) {

  napi_value result, tmp;
  if( napi_ok != napi_create_object( env, &result ) ) return NULL;

  if( napi_ok != napi_create_string_utf8( env, "mix", NAPI_AUTO_LENGTH, &tmp ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "action", tmp ) ) return NULL;

  if( napi_ok != napi_create_string_utf8( env, ev->arg1.c_str(), NAPI_AUTO_LENGTH, &tmp ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "event", tmp ) ) return NULL;

  return result;
}

napi_value createplayobject( napi_env env, projectrtpchannel::pointer p, jschannelevent *ev ) {
  napi_value result, tmp;
  if( napi_ok != napi_create_object( env, &result ) ) return NULL;

  if( napi_ok != napi_create_string_utf8( env, "play", NAPI_AUTO_LENGTH, &tmp ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "action", tmp ) ) return NULL;

  if( napi_ok != napi_create_string_utf8( env, ev->arg1.c_str(), NAPI_AUTO_LENGTH, &tmp ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "event", tmp ) ) return NULL;

  if( napi_ok != napi_create_string_utf8( env, ev->arg2.c_str(), NAPI_AUTO_LENGTH, &tmp ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "reason", tmp ) ) return NULL;

  return result;
}

napi_value createteleventobject( napi_env env, projectrtpchannel::pointer p, jschannelevent *ev ) {
  napi_value result, tmp;
  if( napi_ok != napi_create_object( env, &result ) ) return NULL;

  if( napi_ok != napi_create_string_utf8( env, "telephone-event", NAPI_AUTO_LENGTH, &tmp ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "action", tmp ) ) return NULL;

  if( napi_ok != napi_create_string_utf8( env, ev->arg1.c_str(), NAPI_AUTO_LENGTH, &tmp ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "event", tmp ) ) return NULL;

  return result;
}

/*
In case the user doesn't supply a call back to be notified then
we use this one so that we can pass data back into the threadsafe
enviroment.
*/
static napi_value dummyclose( napi_env env, napi_callback_info info ) {
  return NULL;
}

/*
This function is called when we convert data into node types. This is called
by the napi framework after the threadsafe function has been acquired and called.
*/
static void eventcallback( napi_env env, napi_value jscb, void* context, void* data ) {

  napi_value ourdata = NULL;
  jschannelevent *ev = ( ( jschannelevent * ) data );
  projectrtpchannel::pointer p = ev->p;

  if( "close" == ev->event ) {
    ourdata = createcloseobject( env, p );
  } else if( "record" == ev->event ) {
    ourdata = createrecordobject( env, p, ev );
  } else if( "play" == ev->event ) {
    ourdata = createplayobject( env, p, ev );
  } else if( "telephone-event" == ev->event ) {
    ourdata = createteleventobject( env, p, ev );
  } else if( "mix" == ev->event ) {
    ourdata = createmixobject( env, p, ev );
  }

  napi_value undefined;
  napi_get_undefined( env, &undefined );
  napi_call_function( env,
                      undefined,
                      jscb,
                      1,
                      &ourdata,
                      NULL );

  /* our final call - allow js to clean up */
  if( "close" == ev->event ) {
    napi_threadsafe_function cb = p->cb;
    p->cb = NULL;
    if( napi_ok != napi_release_threadsafe_function( cb, napi_tsfn_abort ) ) {
      fprintf( stderr, "Error releasing threadsafe function\n" );
    }
  }

  delete ev;
}

static napi_value channelcreate( napi_env env, napi_callback_info info ) {

  size_t argc = 2;
  napi_value argv[ 2 ];
  napi_value nremote;

  napi_value result;

  size_t bytescopied;

  int32_t remoteport;
  char remoteaddress[ 128 ];
  remoteaddress[ 0 ] = 0;

  bool hasit;
  bool dtlsenabled = false, dtlsrequired = false;

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, nullptr, nullptr ) ) return NULL;

  if( argc < 1 || argc > 2 ) {
    napi_throw_error( env, "0", "You must provide 1 or 2 params" );
    return NULL;
  }

  projectrtpchannel::pointer p = projectrtpchannel::create( getavailableport() );

  /* optional - remote */
  dtlssession::mode dtlsmode = dtlssession::none;
  char vfingerprint[ 128 ];
  vfingerprint[ 0 ] = 0;
  uint32_t codecval = 0;
  uint32_t ilbcpt = ILBCPAYLOADTYPE;
  uint32_t rfc2833pt = RFC2833PAYLOADTYPE;

  if( napi_ok == napi_has_named_property( env, argv[ 0 ], "remote", &hasit ) &&
      hasit &&
      napi_ok == napi_get_named_property( env, argv[ 0 ], "remote", &nremote ) ) {
    /* 
    optional - DTLS 
    "dtls": {
      "fingerprint": {
        "type": "sha-256 - but ignored for now!",
        "hash": "..."
      },
      "mode": "active|passive"
    }
    */
    napi_value dtls, nactpass;
    if ( napi_ok != napi_has_named_property( env, nremote, "dtls", &hasit ) ||
         !hasit ||
         napi_ok != napi_get_named_property( env, nremote, "dtls", &dtls ) ) goto nodtls;

    dtlsrequired = true;

    if( napi_ok != napi_has_named_property( env, dtls, "mode", &hasit ) ||
        !hasit ||
        napi_ok != napi_get_named_property( env, dtls, "mode", &nactpass ) ) goto nodtls;

    char vactpass[ 128 ];
    vactpass[ 0 ] = 0;

    if( napi_ok != napi_get_value_string_utf8( env, nactpass, vactpass, sizeof( vactpass ), &bytescopied ) ) goto nodtls;

    if( std::string( vactpass ) == "passive" ) {
      dtlsmode = dtlssession::pass;
    } else {
      dtlsmode = dtlssession::act;
    }

    napi_value nfingerprint;
    if( napi_ok != napi_has_named_property( env, dtls, "fingerprint", &hasit ) ||
        !hasit ||
        napi_ok != napi_get_named_property( env, dtls, "fingerprint", &nfingerprint ) ) goto nodtls;

    napi_value nhash;
    if( napi_ok != napi_has_named_property( env, nfingerprint, "hash", &hasit ) ||
        !hasit ||
        napi_ok != napi_get_named_property( env, nfingerprint, "hash", &nhash ) ) goto nodtls;

    if( napi_ok == napi_get_value_string_utf8( env, nhash, vfingerprint, sizeof( vfingerprint ), &bytescopied ) ) {
      if( 95 == bytescopied ) {
        dtlsenabled = true;
      }
    }

nodtls:
    if( dtlsrequired && !dtlsenabled ) {
      napi_throw_error( env, "1", "DTLS requested but not possible" );
      return NULL;
    }

    napi_value nport, naddress, ncodec, nicepwd;

    if( napi_ok != napi_get_named_property( env, nremote, "port", &nport ) ) {
      napi_throw_error( env, "1", "Missing port in remote object" );
      return NULL;
    }

    napi_get_value_int32( env, nport, &remoteport );

    if( napi_ok != napi_get_named_property( env, nremote, "codec", &ncodec ) ) {
      napi_throw_error( env, "1", "Missing codec in remote object" );
      return NULL;
    }

    napi_value nptval;
    if( napi_ok == napi_has_named_property( env, nremote, "ilbcpt", &hasit ) &&
        hasit &&
        napi_ok == napi_get_named_property( env, nremote, "ilbcpt", &nptval ) ) {
      napi_get_value_uint32( env, nptval, &ilbcpt );
    }

    if( napi_ok == napi_has_named_property( env, nremote, "rfc2833pt", &hasit ) &&
        hasit &&
        napi_ok == napi_get_named_property( env, nremote, "rfc2833pt", &nptval ) ) {
      napi_get_value_uint32( env, nptval, &rfc2833pt );
    }

    napi_get_value_uint32( env, ncodec, &codecval );

    if( napi_ok != napi_get_named_property( env, nremote, "address", &naddress ) ) {
      napi_throw_error( env, "1", "Missing address in remote object" );
      return NULL;
    }

    napi_get_value_string_utf8( env, naddress, remoteaddress, sizeof( remoteaddress ), &bytescopied );
    if( 0 == bytescopied || bytescopied >= sizeof( remoteaddress ) ) {
      napi_throw_error( env, "1", "Remote host address too long" );
      return NULL;
    }

    char remoteicepwd[ 128 ];
    remoteicepwd[ 0 ] = 0;
    if( napi_ok == napi_get_named_property( env, nremote, "icepwd", &nicepwd ) ) {
      napi_get_value_string_utf8( env, nicepwd, remoteicepwd, sizeof( remoteicepwd ), &bytescopied );
      if( 0 == bytescopied || bytescopied >= sizeof( remoteaddress ) ) {
        napi_throw_error( env, "1", "Remote ice-pwd too long" );
        return NULL;
      }
      p->iceremotepwd = remoteicepwd;
    }
  }

  /* optional - these have defaults */
  napi_value ndirection;
  if ( napi_ok == napi_has_named_property( env, argv[ 0 ], "direction", &hasit ) &&
       hasit &&
       napi_ok == napi_get_named_property( env, argv[ 0 ], "direction", &ndirection ) ) {

    napi_value nsend, nrecv;
    if( napi_ok == napi_has_named_property( env, ndirection, "send", &hasit ) &&
        hasit &&
        napi_ok == napi_get_named_property( env, ndirection, "send", &nsend ) ) {
      bool vsend;
      if( napi_ok == napi_get_value_bool( env, nsend, &vsend ) ) {
        p->send = vsend;
      }
    }
    if( napi_ok == napi_has_named_property( env, ndirection, "recv", &hasit ) &&
        hasit &&
        napi_ok == napi_get_named_property( env, ndirection, "recv", &nrecv ) ) {
      bool vrecv;
      if( napi_ok == napi_get_value_bool( env, nrecv, &vrecv ) ) {
        p->recv = vrecv;
      }
    }
  }


  napi_value nlocal;
  if( napi_ok == napi_has_named_property( env, argv[ 0 ], "local", &hasit ) &&
      hasit &&
      napi_ok == napi_get_named_property( env, argv[ 0 ], "local", &nlocal ) ) {

    /* optional - override locally generate one */
    napi_value nicepwd;
    if( napi_ok == napi_has_named_property( env, nlocal, "icepwd", &hasit ) &&
          hasit &&
          napi_ok == napi_get_named_property( env, nlocal, "icepwd", &nicepwd ) ) {

      char localicepwd[ 128 ];
      localicepwd[ 0 ] = 0;

      napi_get_value_string_utf8( env, nicepwd, localicepwd, sizeof( localicepwd ), &bytescopied );
      if( 0 == bytescopied || bytescopied >= sizeof( remoteaddress ) ) {
        napi_throw_error( env, "1", "Local ice-pwd too long" );
        return NULL;
      }
      p->icelocalpwd = localicepwd;
    }
  }


  hiddensharedptr *pb = new hiddensharedptr( p );

  napi_value callback;
  if( 1 == argc ) {
    if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, dummyclose, nullptr, &callback ) ) {
      delete pb;
      return NULL;
    }
  } else {
    callback = argv[ 1 ];
  }

  napi_value workname;

  /*
    We don't need to call napi_acquire_threadsafe_function as we are setting
    the inital value of initial_thread_count to 1.
  */
  if( napi_ok != napi_create_string_utf8( env, "projectrtp", NAPI_AUTO_LENGTH, &workname ) ||
      napi_ok != napi_create_threadsafe_function( env,
                                       callback,
                                       NULL,
                                       workname,
                                       0,
                                       1,
                                       NULL,
                                       NULL,
                                       NULL,
                                       eventcallback,
                                       &p->cb ) ||
      napi_ok != napi_create_object( env, &result ) ||
      napi_ok != napi_type_tag_object( env, result, &channelcreatetag ) ||
      napi_ok != napi_wrap( env, result, pb, channeldestroy, pb, nullptr ) ) {
    delete pb;
    return NULL;
  }

  p->remote( remoteaddress, remoteport, codecval, ilbcpt, rfc2833pt, dtlsmode, vfingerprint );
  uint32_t ssrc = p->requestopen();

  /* methods */
  napi_value mfunc;
  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, channelclose, nullptr, &mfunc ) ||
      napi_ok != napi_set_named_property( env, result, "close", mfunc ) ||

      napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, channelmix, nullptr, &mfunc ) ||
      napi_ok != napi_set_named_property( env, result, "mix", mfunc ) ||

      napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, channelunmix, nullptr, &mfunc ) ||
      napi_ok != napi_set_named_property( env, result, "unmix", mfunc ) ||

      napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, channelecho, nullptr, &mfunc ) ||
      napi_ok != napi_set_named_property( env, result, "echo", mfunc ) ||

      napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, channelplay, nullptr, &mfunc ) ||
      napi_ok != napi_set_named_property( env, result, "play", mfunc ) ||

      napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, channelrecord, nullptr, &mfunc ) ||
      napi_ok != napi_set_named_property( env, result, "record", mfunc ) ||

      napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, channeldirection, nullptr, &mfunc ) ||
      napi_ok != napi_set_named_property( env, result, "direction", mfunc ) ||

      napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, channeldtmf, nullptr, &mfunc ) ||
      napi_ok != napi_set_named_property( env, result, "dtmf", mfunc ) ||

      napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, channelremote, nullptr, &mfunc ) ||
      napi_ok != napi_set_named_property( env, result, "remote", mfunc )
    ) {
    delete pb;
    return NULL;
  }

  /* values */
  napi_value nourport, ndtls, fp, nssrc, nicepwd;
  if( napi_ok != napi_create_object( env, &nlocal ) ||
      napi_ok != napi_set_named_property( env, result, "local", nlocal ) ||

      napi_ok != napi_create_int32( env, p->getport(), &nourport ) ||
      napi_ok != napi_set_named_property( env, nlocal, "port", nourport ) ||

      napi_ok != napi_create_uint32( env, ssrc, &nssrc ) ||
      napi_ok != napi_set_named_property( env, nlocal, "ssrc", nssrc ) ||

      napi_ok != napi_create_object( env, &ndtls ) ||
      napi_ok != napi_set_named_property( env, nlocal, "dtls", ndtls ) ||

      napi_ok != napi_create_string_utf8( env, getdtlssrtpsha256fingerprint(), NAPI_AUTO_LENGTH, &fp ) ||
      napi_ok != napi_set_named_property( env, ndtls, "fingerprint", fp ) ||
      napi_ok != napi_set_named_property( env, ndtls, "enabled", createnapibool( env, dtlsenabled ) ) ||
      
      napi_ok != napi_create_string_utf8( env, p->icelocalpwd.c_str(), NAPI_AUTO_LENGTH, &nicepwd ) ||
      napi_ok != napi_set_named_property( env, nlocal, "icepwd", nicepwd ) ) {

    delete pb;
    return NULL;
  }

  return result;
}

void getchannelstats( napi_env env, napi_value &result ) {

  napi_value channel;
  if( napi_ok != napi_create_object( env, &channel ) ) return;
  if( napi_ok != napi_set_named_property( env, result, "channel", channel ) ) return;

  napi_value av, chcount;

  if( napi_ok != napi_create_double( env, getvailableportsize(), &av ) ) return;
  if( napi_ok != napi_create_double( env, channelscreated.load(), &chcount ) ) return;

  if( napi_ok != napi_set_named_property( env, channel, "available", av ) ) return;
  if( napi_ok != napi_set_named_property( env, channel, "current", chcount ) ) return;
}

void initrtpchannel( napi_env env, napi_value &result, int32_t startport, int32_t endport ) {
  napi_value ccreate;

  {
    SpinLockGuard guard( availableportslock );

    while(!availableports.empty()) availableports.pop();
    for( int i = (int) startport; i < (int) endport; i = i + 2 ) {
      availableports.push( i );
    }
  }

  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, channelcreate, nullptr, &ccreate ) ) return;
  if( napi_ok != napi_set_named_property( env, result, "openchannel", ccreate ) ) return;
}

#endif /* NODE_MODULE */
