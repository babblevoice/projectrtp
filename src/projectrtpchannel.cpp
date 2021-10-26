

#include <iostream>
#include <cstdlib>

#include <boost/bind/bind.hpp>
#include <boost/chrono.hpp>
#include <iomanip>
#include <utility>

#include <queue>

#include "projectrtpchannel.h"

extern boost::asio::io_context workercontext;
std::queue < unsigned short >availableports;
std::atomic_bool availableportslock( false );
uint32_t channelscreated = 0;

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
  outpkcount( 0 ),
  outpkskipcount( 0 ),
  inbuff( rtpbuffer::create() ),
  rtpbufferlock( false ),
  rtpoutindex( 0 ),
#ifdef NODE_MODULE
  jsthis( NULL ),
  cb( NULL ),
#endif
  /* private */
  active( false ),
  port( port ),
  rfc2833pt( 101 ),
  lasttelephoneevent( 0 ),
  resolver( workercontext ),
  rtpsocket( workercontext ),
  rtcpsocket( workercontext ),
  rtpsenderendpoint(),
  confirmedrtpsenderendpoint(),
  rtcpsenderendpoint(),
  receivedrtp( false ),
  targetconfirmed( false ),
  mixerlock( false ),
  mixer( nullptr ),
  mixing( false ),
  removemixer( false ),
  outcodec(),
  incodec(),
  player( nullptr ),
  newplaydef( nullptr ),
  newplaylock( false ),
  doecho( false ),
  tick( workercontext ),
  nexttick( std::chrono::high_resolution_clock::now() ),
  newrecorders(),
  newrecorderslock( false ),
  recorders(),
  rtpdtls( nullptr ),
  rtpdtlslock( false ),
  targetaddress(),
  targetport( 0 ),
  queueddigits(),
  queuddigitslock( false ),
  lastdtmfsn( 0 ),
  tickstarttime() {

  channelscreated++;
}


/*
## requestopen
*/
void projectrtpchannel::requestopen( void ) {
  boost::asio::post( workercontext,
        boost::bind( &projectrtpchannel::doopen, shared_from_this() ) );
}

void projectrtpchannel::requestclose( std::string reason ) {
  if( !this->_requestclose.exchange( true, std::memory_order_acquire ) ) {
    this->closereason = reason;
  }
}

void projectrtpchannel::target( std::string address,
                                unsigned short port,
                                uint32_t codec,
                                dtlssession::mode m,
                                std::string fingerprint ) {
  this->targetconfirmed = false;
  this->targetaddress = address;
  this->targetport = port;
  this->codec = codec;

  if( dtlssession::none != m ) {
    dtlssession::pointer newsession = dtlssession::create( m );
    newsession->setpeersha256( fingerprint );

    projectrtpchannel::pointer p = shared_from_this();
    newsession->ondata( [ p ] ( const void *d , size_t l ) -> void {
      /* Note to me, I need to confirm that gnutls maintains the buffer ptr until after the handshake is confirmed (or
         at least until we have sent the packet). */
      if( p->targetconfirmed ) {
        p->rtpsocket.async_send_to(
                          boost::asio::buffer( d, l ),
                          p->confirmedrtpsenderendpoint,
                          []( const boost::system::error_code& ec, std::size_t bytes_transferred ) -> void {
                            /* We don't need to do anything */
                          } );
      }
    } );

    AQUIRESPINLOCK( this->rtpdtlslock );
    this->rtpdtls = newsession;
    RELEASESPINLOCK( this->rtpdtlslock );
  }

  if( this->active ) this->dotarget();
}

/*
Must be called in the workercontext.
*/
void projectrtpchannel::doopen( void ) {

  this->outcodec.reset();
  this->incodec.reset();

  boost::asio::socket_base::reuse_address reuseoption( true );
  this->rtpsocket.open( boost::asio::ip::udp::v4() );
  this->rtpsocket.set_option( reuseoption );
  this->rtpsocket.bind( boost::asio::ip::udp::endpoint(
      boost::asio::ip::udp::v4(), this->port ) );

  this->rtcpsocket.open( boost::asio::ip::udp::v4() );
  this->rtcpsocket.set_option( reuseoption );
  this->rtcpsocket.bind( boost::asio::ip::udp::endpoint(
      boost::asio::ip::udp::v4(), this->port + 1 ) );

  if( !this->rtpsocket.is_open() || !this->rtcpsocket.is_open() ) {
    fprintf( stderr, "No more sockets available - refusing a new channel\n" );
    this->requestclose( "error.nosocket" );
    this->doclose();
    return;
  }

  this->active = true;

  this->ssrcout = rand();

  /* anchor our out time to when the channel is opened */
  this->tsout = std::chrono::system_clock::to_time_t( std::chrono::system_clock::now() );
  this->snout = rand();

  if( 0 != this->targetport ) this->dotarget();

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

  AQUIRESPINLOCK( availableportslock );
  availableports.push( this->getport() );
  RELEASESPINLOCK( availableportslock );

  channelscreated--;
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

void projectrtpchannel::doclose( void ) {

  if( !this->active ) return;

  this->active = false;
  this->tick.cancel();

  if( nullptr != this->player ) {
    postdatabacktojsfromthread( shared_from_this(), "play", "end", "channelclosed" );
  }

  this->player = nullptr;
  this->newplaydef = nullptr;

  this->mixer = nullptr;
  this->mixing = false;
  this->removemixer = true;

  /* close our session if we have one */
  this->rtpdtls = nullptr;

  /* close up any remaining recorders */
  for( auto& rec: this->recorders ) {
    postdatabacktojsfromthread( shared_from_this(), "record", rec->file, "finished.channelclosed" );
  }

  this->recorders.clear();
  this->rtpsocket.cancel();
  this->rtcpsocket.cancel();
  this->resolver.cancel();

  this->rtpsocket.close();
  this->rtcpsocket.close();

  postdatabacktojsfromthread( shared_from_this(), "close", this->closereason );
}

bool projectrtpchannel::checkidlerecv( void ) {
  if( this->recv && this->active ) {
    this->tickswithnortpcount++;
    if( this->tickswithnortpcount > ( 50 * 20 ) ) { /* 50 (@20mS ptime)/S = 20S */
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
  if ( error == boost::asio::error::operation_aborted ) return;

  if( this->_requestclose ) {
    this->doclose();
    return;
  }

  if( this->mixing ) {
    this->setnexttick();
    return;
  };

  this->startticktimer();

  if( this->dtlsnegotiate() ) {
    this->endticktimer();
    this->setnexttick();
    return;
  }

  this->incrtsout();
  if( this->checkidlerecv() ) return;
  this->checkfornewrecorders();

  this->incodec << codecx::next;
  this->outcodec << codecx::next;

  AQUIRESPINLOCK( this->rtpdtlslock );
  dtlssession::pointer currentdtlssession = this->rtpdtls;
  RELEASESPINLOCK( this->rtpdtlslock );

  rtppacket *src;
  do {
    AQUIRESPINLOCK( this->rtpbufferlock );
    src = this->inbuff->pop();
    RELEASESPINLOCK( this->rtpbufferlock );

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

  /* check for new players */
  {
    bool playerreplaced = false;
    bool playernew = false;
    AQUIRESPINLOCK( this->newplaylock );
    if( nullptr != this->newplaydef ) {
      if( nullptr != this->player ) {
        playerreplaced = true;
      }

      this->player = this->newplaydef;
      this->newplaydef = nullptr;
      playernew = true;
    }
    RELEASESPINLOCK( this->newplaylock );

    if( playerreplaced ) {
      postdatabacktojsfromthread( shared_from_this(), "play", "end", "replaced" );
    }
    if( playernew ) {
      postdatabacktojsfromthread( shared_from_this(), "play", "start", "new" );
    }
  }

  if( this->player ) {
    rtppacket *out = this->gettempoutbuf();
    rawsound r;
    if( this->player->read( r ) ) {
      if( r.size() > 0 ) {
        this->outcodec << r;
        out << this->outcodec;
        this->writepacket( out );
      }
    } else {
      this->player = nullptr;
      postdatabacktojsfromthread( shared_from_this(), "play", "end", "completed" );
    }
  } else if( this->doecho ) {
    if( nullptr != src ) {
      this->outcodec << *src;

      rtppacket *dst = this->gettempoutbuf();
      dst << this->outcodec;
      this->writepacket( dst );
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
  AQUIRESPINLOCK( this->newplaylock );
  this->newplaydef = newdef;
  RELEASESPINLOCK( this->newplaylock );
}

/*
Post a request to record (or modify a param of a record). This function is typically
called from a control thread (i.e. node).
*/
void projectrtpchannel::requestrecord( channelrecorder::pointer rec ) {
  AQUIRESPINLOCK( this->newrecorderslock );
  this->newrecorders.push_back( rec );
  RELEASESPINLOCK( this->newrecorderslock );
}

/*
Check for new channels to add to the mix in our own thread. This will be a different thread
to requestrecord.
*/
void projectrtpchannel::checkfornewrecorders( void ) {
  channelrecorder::pointer rec;
  AQUIRESPINLOCK( this->newrecorderslock );

  for ( auto const& newrec : this->newrecorders ) {

    for( auto& currentrec: this->recorders ) {
      if( currentrec->file == newrec->file ) {
        currentrec->pause = newrec->pause;
        currentrec->requestfinish = newrec->requestfinish;
        goto endofwhileloop;
      }
    }

    newrec->sfile = soundfilewriter::create(
        newrec->file,
        soundfile::wavformatfrompt( this->codec ),
        newrec->numchannels,
        soundfile::getsampleratefrompt( this->codec ) );

    this->recorders.push_back( newrec );

endofwhileloop:;
  }

  this->newrecorders.clear();

  RELEASESPINLOCK( this->newrecorderslock );
}

void projectrtpchannel::removeoldrecorders( void ) {

  for ( auto const& rec : this->recorders ) {
    if( rec->isactive() ) {

      boost::posix_time::ptime nowtime( boost::posix_time::microsec_clock::local_time() );
      boost::posix_time::time_duration const diff = ( nowtime - rec->activeat );

      if( diff.total_milliseconds() < rec->minduration  ) {
        continue;
      }

      if( rec->requestfinish ) {
        rec->completed = true;
        postdatabacktojsfromthread( shared_from_this(), "record", rec->file, "finished.requested" );
        continue;
      }

      if( rec->lastpowercalc < rec->finishbelowpower ) {
        rec->completed = true;
        postdatabacktojsfromthread( shared_from_this(), "record", rec->file, "finished.belowpower" );
        continue;
      }

      if( 0 != rec->maxduration && diff.total_milliseconds() > rec->maxduration ) {
        rec->completed = true;
        postdatabacktojsfromthread( shared_from_this(), "record", rec->file, "finished.timeout" );
        continue;
      }
    }
  }

  for ( chanrecptrlist::iterator rec = this->recorders.begin();
        rec != this->recorders.end(); ) {
    if( ( *rec )->completed ) {
      rec = this->recorders.erase( rec );
    } else {
      ++rec;
    }
  }
}

/*
## checkfordtmf

We should receive a start packet with mark set to true. This should then continue until a packet with an
end of event marked in the 2833 payload. But we might lose any one of these packets and should still work
if we do.
*/
static char dtmfchars[] = { '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '*', '#', 'A', 'B', 'C', 'D', 'F' };
bool projectrtpchannel::checkfordtmf( rtppacket *src ) {
  /* The next section is sending to our recipient(s) */
  if( nullptr != src &&
      0 != this->rfc2833pt &&
      src->getpayloadtype() == this->rfc2833pt ) {

    if( src->getpayloadlength() >= 4 ) {
      /* We have to look for DTMF events handling issues like missing events - such as the marker or end bit */
      uint16_t sn = src->getsequencenumber();

      uint8_t * pl = src->getpayload();
      uint8_t endbit = pl[ 1 ] >> 7;
      uint8_t event = pl[ 0 ] & 0x7f;

      if( event <= 16 ) {
        uint8_t pm = src->getpacketmarker();

        /* Have we lost our mark packet */
        if( 0 == pm &&
            0 == this->lasttelephoneevent ) {
          pm = 1;
        }

        /* did we lose the last end of event */
        if( 0 != this->lasttelephoneevent &&
            abs( static_cast< long long int >( sn - this->lasttelephoneevent ) ) > 20 ) {
          pm = 1;
        }

        if( 1 == pm ) {
          postdatabacktojsfromthread( shared_from_this(), "telephone-event", std::string( 1, dtmfchars[ event ] ) );
        }

        if( endbit ) {
          this->lasttelephoneevent = 0;
        } else {
          this->lasttelephoneevent = sn;
        }
      }
    }
    return true;
  }
  return false;
}


/*
# writerecordings
If our codecs (in and out) have data then write to recorded files.
*/
void projectrtpchannel::writerecordings( void ) {

  if( 0 == this->recorders.size() ) return;
  uint16_t power = 0;

  /* Decide if we need to calc power as it is expensive */
  for( auto& rec: this->recorders ) {
    if( ( !rec->isactive() && 0 != rec->startabovepower ) || 0 != rec->finishbelowpower ) {
      power = this->incodec.power();
      break;
    }
  }

  /* Check if we need to trigger the start of any recordings and write */
  for( auto& rec: this->recorders ) {

    if( rec->completed ) continue;

    if( 0 == rec->startabovepower && 0 == rec->finishbelowpower ) {
      if( !rec->isactive() ) {
        rec->active();
        postdatabacktojsfromthread( shared_from_this(), "record", rec->file, "recording" );
      }
    } else {
      auto pav = rec->poweravg( power );
      if( !rec->isactive() && pav > rec->startabovepower ) {
        rec->active();
        postdatabacktojsfromthread( shared_from_this(), "record", rec->file, "recording" );
      }
    }

    if( rec->isactive() && !rec->pause ) {
      rec->sfile->write( this->incodec, this->outcodec );
    }
  }

  this->removeoldrecorders();
}

bool projectrtpchannel::dtlsnegotiate( void ) {

  AQUIRESPINLOCK( this->rtpdtlslock );
  dtlssession::pointer oursession = this->rtpdtls;
  RELEASESPINLOCK( this->rtpdtlslock );

  if( nullptr == oursession ) return false;
  if( !oursession->rtpdtlshandshakeing ) return false;

  AQUIRESPINLOCK( this->rtpdtlslock );
  auto dtlsstate = oursession->handshake();
  RELEASESPINLOCK( this->rtpdtlslock );

  if( GNUTLS_E_SUCCESS == dtlsstate ) {
    oursession->rtpdtlshandshakeing = false;
  }

  return oursession->rtpdtlshandshakeing;
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

  this->rtpsocket.cancel();
  this->rtpsocket.async_wait(
    boost::asio::ip::tcp::socket::wait_read,
    [ this ]( const boost::system::error_code& error ) {

      if( boost::asio::error::operation_aborted == error ) {
        return;
      }

      AQUIRESPINLOCK( this->rtpdtlslock );
      dtlssession::pointer currentdtlssession = this->rtpdtls;
      RELEASESPINLOCK( this->rtpdtlslock );

      if( nullptr != currentdtlssession &&
          currentdtlssession->rtpdtlshandshakeing ) {
        /* DTLS Handshake */
        uint8_t dtlsbuf[ 1500 ];
        auto bytesrecvd = this->rtpsocket.receive_from( boost::asio::buffer( dtlsbuf, sizeof( dtlsbuf ) ), this->rtpsenderendpoint );

        if( bytesrecvd > 0 ) {
          AQUIRESPINLOCK( this->rtpdtlslock );
          currentdtlssession->write( dtlsbuf, bytesrecvd );
          RELEASESPINLOCK( this->rtpdtlslock );
          this->readsomertp();
        }

      } else {
        /* RTP */
        /* Grab a buffer */
        AQUIRESPINLOCK( this->rtpbufferlock );
        rtppacket* buf = this->inbuff->reserve();
        RELEASESPINLOCK( this->rtpbufferlock );
        if( nullptr == buf ) {
          this->requestclose( "error.nobuffer" );
          return;
        }

        auto bytesrecvd = this->rtpsocket.receive_from( boost::asio::buffer( buf->pk, RTPMAXLENGTH ), this->rtpsenderendpoint );

        if( RTPMAXLENGTH == bytesrecvd ) {
          // Too large
          this->receivedpkcount++;
          this->receivedpkskip++;
          this->readsomertp();
        } else if( bytesrecvd > 0 ) {

          /* We should still count packets we are instructed to drop */
          this->receivedpkcount++;

          if( !this->recv ) {
            if( bytesrecvd > 0 && this->active ) {
              this->readsomertp();
            }
            return;
          }

          this->tickswithnortpcount = 0;

          if( !this->receivedrtp ) {
            this->confirmedrtpsenderendpoint = this->rtpsenderendpoint;
            this->receivedrtp = true;
            this->ssrcin = buf->getssrc();
          } else {
            /* After the first packet - we only accept data from the verified source */
            if( this->confirmedrtpsenderendpoint != this->rtpsenderendpoint ) {
              this->readsomertp();
              return;
            }

            if( buf->getssrc() != this->ssrcin ) {
              this->receivedpkskip++;
              this->readsomertp();
              return;
            }
          }

          buf->length = bytesrecvd;

          AQUIRESPINLOCK( this->rtpbufferlock );
          this->inbuff->push();
          RELEASESPINLOCK( this->rtpbufferlock );


          this->readsomertp();
        } else {
          this->receivedpkcount++;
          this->receivedpkskip++;
        }
      }
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

  if( nullptr == pk ) {
    this->outpkskipcount++;
    fprintf( stderr, "We have been given an nullptr RTP packet??\n" );
    return;
  }

  if( 0 == pk->length ) {
    this->outpkskipcount++;
    fprintf( stderr, "We have been given an RTP packet of zero length??\n" );
    return;
  }

  if( !this->send ) {
    this->outpkskipcount++;
    /* silently drop - could we do this sooner to use less CPU? */
    return;
  }

  AQUIRESPINLOCK( this->rtpdtlslock );
  dtlssession::pointer currentdtlssession = this->rtpdtls;
  RELEASESPINLOCK( this->rtpdtlslock );

  if( nullptr != currentdtlssession &&
      currentdtlssession->rtpdtlshandshakeing ) {
    this->outpkskipcount++;
    return;
  }

  if( nullptr != currentdtlssession &&
      !currentdtlssession->rtpdtlshandshakeing ) {
    if( !currentdtlssession->protect( pk ) ) {
      this->outpkskipcount++;
      return;
    }
  }

  if( this->receivedrtp || this->targetconfirmed ) {
    this->snout++;

    this->rtpsocket.async_send_to(
                      boost::asio::buffer( pk->pk, pk->length ),
                      this->confirmedrtpsenderendpoint,
                      boost::bind( &projectrtpchannel::handlesend,
                                    shared_from_this(),
                                    boost::asio::placeholders::error,
                                    boost::asio::placeholders::bytes_transferred ) );
  }
}

void projectrtpchannel::dotarget( void ) {
  if( "" == this->targetaddress ) return;

  this->receivedrtp = false;
  boost::asio::ip::udp::resolver::query query(
    boost::asio::ip::udp::v4(),
    this->targetaddress,
    std::to_string( this->targetport ) );

  /* Resolve the address */
  this->resolver.async_resolve( query,
      boost::bind( &projectrtpchannel::handletargetresolve,
        shared_from_this(),
        boost::asio::placeholders::error,
        boost::asio::placeholders::iterator ) );
}

/*!md
## handletargetresolve
We have resolved the target address and port now use it. Further work could be to inform control there is an issue.
*/
void projectrtpchannel::handletargetresolve (
            boost::system::error_code e,
            boost::asio::ip::udp::resolver::iterator it ) {
  boost::asio::ip::udp::resolver::iterator end;

  if( e == boost::asio::error::operation_aborted ) return;

  if( it == end ) {
    /* Failure - silent (the call will be as well!) */
    this->requestclose( "failed.target" );
    return;
  }

  this->confirmedrtpsenderendpoint = *it;
  this->targetconfirmed = true;

  /* allow us to re-auto correct */
  this->receivedrtp = false;
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

  AQUIRESPINLOCK( this->mixerlock );

  if( nullptr == this->mixer && nullptr != other->mixer ) {
    this->mixer = other->mixer;
    this->mixer->addchannel( shared_from_this() );

  } else if ( nullptr != this->mixer && nullptr == other->mixer ) {
    other->mixer = this->mixer;
    this->mixer->addchannel( other );

  } else if( nullptr == this->mixer && nullptr == other->mixer  ) {
    this->mixer = projectchannelmux::create( workercontext );
    other->mixer = this->mixer;

    this->mixer->addchannels( shared_from_this(), other );
  } else {
    /* If we get here this and other are already mixing and should be cleaned up first */
    RELEASESPINLOCK( this->mixerlock );
    return false;
  }

  this->mixing = true;
  other->mixing = true;

  this->mixer->go();

  RELEASESPINLOCK( this->mixerlock );

  return true;
}

/*
## mix
Add the other to a mixer - both channels have access to the same mixer.
n way relationship. Adds to queue for when our main thread calls into us.
*/
bool projectrtpchannel::unmix( void ) {
  this->removemixer = true;
  return true;
}

/*
## dtmf
Queue digits to send as RFC 2833.
*/
void projectrtpchannel::dtmf( std::string digits ) {
  AQUIRESPINLOCK( this->queuddigitslock );
  this->queueddigits += digits;
  RELEASESPINLOCK( this->queuddigitslock );
}

/*
Now send each digit.
*/
void projectrtpchannel::senddtmf( void ) {

  if( static_cast< uint16_t >( this->snout - this->lastdtmfsn ) < 10 ) {
    return;
  }

  uint8_t tosend = 0;
  AQUIRESPINLOCK( this->queuddigitslock );
  if( this->queueddigits.size() > 0 ) {
    tosend = this->queueddigits[ 0 ];
    this->queueddigits.erase( this->queueddigits.begin() );
  }
  RELEASESPINLOCK( this->queuddigitslock );

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

  rtppacket *dst = this->gettempoutbuf();
  dst->setpayloadtype( this->rfc2833pt );
  dst->setmarker();
  dst->setpayloadlength( 4 );
  uint8_t *pl =  dst->getpayload();
  pl[ 0 ] = tosend;
  pl[ 1 ] = 10; /* end of event & reserved & volume */
  pl[ 2 ] = 0; /* event duration high */
  pl[ 3 ] = 160; /* event duration */
  this->writepacket( dst );

  dst = this->gettempoutbuf();
  dst->setpayloadtype( this->rfc2833pt );
  dst->setpayloadlength( 4 );
  pl =  dst->getpayload();
  pl[ 0 ] = tosend;
  pl[ 1 ] = 10; /* end of event & reserved & volume */
  pl[ 2 ] = 0; /* event duration high */
  pl[ 3 ] = 160; /* event duration */
  this->writepacket( dst );

  dst = this->gettempoutbuf();
  dst->setpayloadtype( this->rfc2833pt );
  dst->setpayloadlength( 4 );
  pl =  dst->getpayload();
  pl[ 0 ] = tosend;
  pl[ 1 ] = 0x80 | 10; /* end of event & reserved & volume */
  pl[ 2 ] = 0; /* event duration high */
  pl[ 3 ] = 160; /* event duration */
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
   "poweraveragepackets": 50
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
static napi_value channeltarget( napi_env env, napi_callback_info info ) {

  size_t argc = 1;
  napi_value argv[ 1 ];

  napi_value nport, naddress, ncodec;
  int32_t targetport;

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

  napi_get_value_int32( env, nport, &targetport );

  if( napi_ok != napi_get_named_property( env, argv[ 0 ], "address", &naddress ) ) {
    return createnapibool( env, false );
  }

  if( napi_ok != napi_get_named_property( env, argv[ 0 ], "codec", &ncodec ) ) {
    return createnapibool( env, false );
  }

  uint32_t codecval = 0;
  napi_get_value_uint32( env, ncodec, &codecval );

  size_t bytescopied;
  char targetaddress[ 128 ];

  napi_get_value_string_utf8( env, naddress, targetaddress, sizeof( targetaddress ), &bytescopied );
  if( 0 == bytescopied || bytescopied >= sizeof( targetaddress ) ) {
    return createnapibool( env, false );
  }

  /* optional - DTLS */
  napi_value dtls;
  bool hasit;
  dtlssession::mode dtlsmode = dtlssession::none;
  char vfingerprint[ 128 ];
  vfingerprint[ 0 ] = 0;

  if ( napi_ok == napi_has_named_property( env, argv[ 0 ], "dtls", &hasit ) &&
       hasit &&
       napi_ok == napi_get_named_property( env, argv[ 0 ], "dtls", &dtls ) ) {
    napi_value nfingerprint, nactpass;
    if( napi_ok == napi_has_named_property( env, dtls, "fingerprint", &hasit ) &&
        hasit &&
        napi_ok == napi_get_named_property( env, dtls, "fingerprint", &nfingerprint ) ) {
      size_t bytescopied;
      char vactpass[ 128 ];
      if( napi_ok == napi_get_value_string_utf8( env, nfingerprint, vfingerprint, sizeof( vfingerprint ), &bytescopied ) ) {
        if( napi_ok == napi_has_named_property( env, dtls, "mode", &hasit ) &&
            hasit &&
            napi_ok == napi_get_named_property( env, dtls, "mode", &nactpass ) ) {
          if( napi_ok == napi_get_value_string_utf8( env, nactpass, vactpass, sizeof( vactpass ), &bytescopied ) ) {
            if( std::string( vactpass ) == "pass" ) {
              /* If they are pass - we are act */
              dtlsmode = dtlssession::act;
            } else {
              dtlsmode = dtlssession::pass;
            }
          }
        }
      }
    }
  }

  chan->target( targetaddress, targetport, codecval, dtlsmode, vfingerprint );

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

  napi_value receivedpkcount, receivedpkskip, receiveddropped;
  if( napi_ok != napi_create_double( env, p->receivedpkcount, &receivedpkcount ) ) return NULL;
  if( napi_ok != napi_create_double( env, p->receivedpkskip, &receivedpkskip ) ) return NULL;
  if( napi_ok != napi_create_double( env, p->inbuff->getdropped(), &receiveddropped ) ) return NULL;

  if( napi_ok != napi_set_named_property( env, in, "mos", mos ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, in, "count", receivedpkcount ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, in, "dropped", receiveddropped ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, in, "skip", receivedpkskip ) ) return NULL;

  napi_value sentpkcount, outpkskipcount;
  if( napi_ok != napi_create_double( env, p->outpkcount, &sentpkcount ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, out, "count", sentpkcount ) ) return NULL;

  if( napi_ok != napi_create_double( env, p->outpkskipcount, &outpkskipcount ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, out, "skip", outpkskipcount ) ) return NULL;

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
    napi_release_threadsafe_function( p->cb, napi_tsfn_abort );
  }

  delete ev;
}

static napi_value channelcreate( napi_env env, napi_callback_info info ) {

  size_t argc = 2;
  napi_value argv[ 2 ];
  napi_value ntarget, nport, naddress, ncodec;

  napi_value result;
  AQUIRESPINLOCK( availableportslock );
  auto ourport = availableports.front();
  availableports.pop();
  RELEASESPINLOCK( availableportslock );

  int32_t targetport;
  char targetaddress[ 128 ];
  targetaddress[ 0 ] = 0;

  bool hasit;
  bool dtlsenabled = false;

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, nullptr, nullptr ) ) return NULL;

  if( argc < 1 || argc > 2 ) {
    napi_throw_error( env, "0", "You must provide 1 or 2 params" );
    return NULL;
  }

  projectrtpchannel::pointer p = projectrtpchannel::create( ourport );

  /* optional - target */
  dtlssession::mode dtlsmode = dtlssession::none;
  char vfingerprint[ 128 ];
  vfingerprint[ 0 ] = 0;
  uint32_t codecval = 0;

  if( napi_ok == napi_has_named_property( env, argv[ 0 ], "target", &hasit ) &&
      hasit &&
      napi_ok == napi_get_named_property( env, argv[ 0 ], "target", &ntarget ) ) {

    /* optional - DTLS */
    napi_value dtls;
    if ( napi_ok == napi_has_named_property( env, ntarget, "dtls", &hasit ) &&
         hasit &&
         napi_ok == napi_get_named_property( env, ntarget, "dtls", &dtls ) ) {

      napi_value nfingerprint, nactpass;
      if( napi_ok == napi_has_named_property( env, dtls, "fingerprint", &hasit ) &&
          hasit &&
          napi_ok == napi_get_named_property( env, dtls, "fingerprint", &nfingerprint ) ) {

        size_t bytescopied;
        char vactpass[ 128 ];
        vactpass[ 0 ] = 0;
        if( napi_ok == napi_get_value_string_utf8( env, naddress, vfingerprint, sizeof( vfingerprint ), &bytescopied ) ) {
          if( napi_ok == napi_has_named_property( env, dtls, "mode", &hasit ) &&
              hasit &&
              napi_ok == napi_get_named_property( env, dtls, "mode", &nactpass ) ) {
            if( napi_ok == napi_get_value_string_utf8( env, nactpass, vactpass, sizeof( vactpass ), &bytescopied ) ) {
              if( std::string( vactpass ) == "pass" ) {
                /* If they are pass - we are act */
                dtlsmode = dtlssession::act;
              } else {
                dtlsmode = dtlssession::pass;
              }
              dtlsenabled = true;
            }
          }
        }
      }
    }

    if( napi_ok != napi_get_named_property( env, ntarget, "port", &nport ) ) {
      napi_throw_error( env, "1", "Missing port in target object" );
      return NULL;
    }

    napi_get_value_int32( env, nport, &targetport );

    if( napi_ok != napi_get_named_property( env, ntarget, "address", &naddress ) ) {
      napi_throw_error( env, "1", "Missing address in target object" );
      return NULL;
    }

    if( napi_ok != napi_get_named_property( env, ntarget, "codec", &ncodec ) ) {
      napi_throw_error( env, "1", "Missing codec in target object" );
      return NULL;
    }

    napi_get_value_uint32( env, ncodec, &codecval );

    size_t bytescopied;
    napi_get_value_string_utf8( env, naddress, targetaddress, sizeof( targetaddress ), &bytescopied );
    if( 0 == bytescopied || bytescopied >= sizeof( targetaddress ) ) {
      napi_throw_error( env, "1", "Target host address too long" );
      return NULL;
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

  p->jsthis = result;
  p->target( targetaddress, targetport, codecval, dtlsmode, vfingerprint );
  p->requestopen();

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

      napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, channeltarget, nullptr, &mfunc ) ||
      napi_ok != napi_set_named_property( env, result, "target", mfunc )
    ) {
    delete pb;
    return NULL;
  }

  /* values */
  napi_value nlocal, nourport, ndtls, fp;
  if( napi_ok != napi_create_object( env, &nlocal ) ||
      napi_ok != napi_set_named_property( env, result, "local", nlocal ) ||

      napi_ok != napi_create_int32( env, ourport, &nourport ) ||
      napi_ok != napi_set_named_property( env, nlocal, "port", nourport ) ||


      napi_ok != napi_create_object( env, &ndtls ) ||
      napi_ok != napi_set_named_property( env, nlocal, "dtls", ndtls ) ||

      napi_ok != napi_create_string_utf8( env, getdtlssrtpsha256fingerprint(), NAPI_AUTO_LENGTH, &fp ) ||
      napi_ok != napi_set_named_property( env, ndtls, "fingerprint", fp ) ||
      napi_ok != napi_set_named_property( env, ndtls, "enabled", createnapibool( env, dtlsenabled ) ) ) {

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

  AQUIRESPINLOCK( availableportslock );
  auto availableportssize = availableports.size();
  RELEASESPINLOCK( availableportslock );

  if( napi_ok != napi_create_double( env, availableportssize, &av ) ) return;
  if( napi_ok != napi_create_double( env, channelscreated, &chcount ) ) return;

  if( napi_ok != napi_set_named_property( env, channel, "available", av ) ) return;
  if( napi_ok != napi_set_named_property( env, channel, "current", chcount ) ) return;
}

void initrtpchannel( napi_env env, napi_value &result ) {
  napi_value ccreate;

  for( int i = 10000; i < 20000; i = i + 2 ) {
    availableports.push( i );
  }

  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, channelcreate, nullptr, &ccreate ) ) return;
  if( napi_ok != napi_set_named_property( env, result, "openchannel", ccreate ) ) return;
}

#endif /* NODE_MODULE */
