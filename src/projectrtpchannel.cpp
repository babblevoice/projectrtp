

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
uint32_t channelscreated = 0;

/* The one function used by our channel */
void postdatabacktojsfromthread( projectrtpchannel::pointer p, std::string event, std::string arg1 = "", std::string arg2 = "" );

/*
# Project RTP Channel

This file (class) represents an RP channel. That is an RTP stream (UDP) with
its pair RTCP socket. Basic functions for

1. Opening and closing channels
2. bridging 2 channels
3. Sending data to an endpoint based on us receiving data first or (to be
implimented) the address and port given to us when opening in the channel.

*/
projectrtpchannel::projectrtpchannel( unsigned short port ):
  /* public */
  requestclose( false ),
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
  inbuff( rtpbuffer::create( BUFFERPACKETCOUNT, BUFFERPACKETCAP ) ),
  rtpbufferlock( false ),
  rtpoutindex( 0 ),
  jsthis( NULL ),
  cb( NULL ),
  /* private */
  active( false ),
  port( port ),
  rfc2833pt( 0 ),
  lasttelephoneevent( 0 ),
  resolver( workercontext ),
  rtpsocket( workercontext ),
  rtcpsocket( workercontext ),
  receivedrtp( false ),
  targetconfirmed( false ),
  mixing( false ),
  player( nullptr ),
  newplaydef( nullptr ),
  newplaylock( false ),
  doecho( false ),
  tick( workercontext ),
  newrecorderslock( false ),
  rtpdtls( nullptr ),
  rtpdtlshandshakeing( false ) {

  channelscreated++;
}


/*
## requestopen
*/
void projectrtpchannel::requestopen( std::string address, unsigned short port, uint32_t codec ) {

  this->targetaddress = address;
  this->targetport = port;
  this->codec = codec;

  boost::asio::post( workercontext,
        boost::bind( &projectrtpchannel::doopen, shared_from_this() ) );
}
/*
Must be called in the workercontext.
*/
void projectrtpchannel::doopen( void ) {

  this->outcodec.reset();
  this->incodec.reset();

  this->rtpsocket.open( boost::asio::ip::udp::v4() );
  this->rtpsocket.bind( boost::asio::ip::udp::endpoint(
      boost::asio::ip::udp::v4(), this->port ) );

  this->rtcpsocket.open( boost::asio::ip::udp::v4() );
  this->rtcpsocket.bind( boost::asio::ip::udp::endpoint(
      boost::asio::ip::udp::v4(), this->port + 1 ) );

  this->active = true;

  this->ssrcout = rand();

  /* anchor our out time to when the channel is opened */
  this->tsout = std::chrono::system_clock::to_time_t( std::chrono::system_clock::now() );
  this->snout = rand();

  this->dotarget();

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
  channelscreated--;
}

/*
# create

*/
projectrtpchannel::pointer projectrtpchannel::create( unsigned short port ) {
  return pointer( new projectrtpchannel( port ) );
}

void projectrtpchannel::enabledtls( dtlssession::mode m, std::string &fingerprint ) {
  this->rtpdtls = dtlssession::create( m );
  this->rtpdtls->setpeersha256( fingerprint );

  this->rtpdtlshandshakeing = true;

  projectrtpchannel::pointer p = shared_from_this();
  this->rtpdtls->ondata( [ p ] ( const void *d , size_t l ) -> void {
    /* Note to me, I need to confirm that gnutls maintains the buffer ptr until after the handshake is confirmed (or
       at least until we have sent the packet). */
    p->rtpsocket.async_send_to(
                      boost::asio::buffer( d, l ),
                      p->confirmedrtpsenderendpoint,
                      []( const boost::system::error_code& ec, std::size_t bytes_transferred ) -> void {
                        /* We don't need to do anything */
                      } );

  } );
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

  /* close our session if we have one */
  this->rtpdtls = nullptr;
  this->rtpdtlshandshakeing = false;

  /* close up any remaining recorders */
  for( auto& rec: this->recorders ) {
    postdatabacktojsfromthread( shared_from_this(), "record", rec->file, "finished.channelclosed" );
  }

  this->recorders.clear();
  this->rtpsocket.close();
  this->rtcpsocket.close();
  this->resolver.cancel();

  postdatabacktojsfromthread( shared_from_this(), "close" );
}

bool projectrtpchannel::checkidlerecv( void ) {
  if( this->recv && this->active ) {
    this->tickswithnortpcount++;
    if( this->tickswithnortpcount > ( 50 * 20 ) ) { /* 50 (@20mS ptime)/S = 20S */
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
  if ( error != boost::asio::error::operation_aborted ) {

    if( this->requestclose ) {
      this->doclose();
      return;
    }

    /* calc a timer */
    boost::posix_time::ptime nowtime( boost::posix_time::microsec_clock::local_time() );

    if( !this->mixing ) {
      this->incrtsout();

      if( this->checkidlerecv() ) return;
      this->checkfornewrecorders();

      this->incodec << codecx::next;
      this->outcodec << codecx::next;

      AQUIRESPINLOCK( this->rtpbufferlock );
      rtppacket *src = this->inbuff->pop();
      RELEASESPINLOCK( this->rtpbufferlock );

      if( nullptr != src ) {
        this->incodec << *src;
      }

      AQUIRESPINLOCK( this->newplaylock );
      if( nullptr != this->newplaydef ) {
        if( nullptr != this->player ) {
          postdatabacktojsfromthread( shared_from_this(), "play", "end", "replaced" );
        }

        this->player = this->newplaydef;
        this->newplaydef = nullptr;

        postdatabacktojsfromthread( shared_from_this(), "play", "start", "new" );
      }
      RELEASESPINLOCK( this->newplaylock );

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

      boost::posix_time::time_duration const diff = ( boost::posix_time::microsec_clock::local_time() - nowtime );
      uint64_t tms = diff.total_microseconds();
      this->totalticktime += tms;
      this->totaltickcount++;
      if( tms > this->maxticktime ) this->maxticktime = tms;
    }

    /* The last thing we do */
    this->nexttick = this->nexttick + std::chrono::milliseconds( 20 );

    this->tick.expires_after( this->nexttick - std::chrono::high_resolution_clock::now() );
    this->tick.async_wait( boost::bind( &projectrtpchannel::handletick,
                                        shared_from_this(),
                                        boost::asio::placeholders::error ) );
  }
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

    newrec->sfile = soundfile::create(
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

/*
## handlereadsomertp
Wait for RTP data. We have to re-order when required. Look after all of the round robin memory here.
We should have enough time to deal with the in data before it gets overwritten.

WARNING WARNING
Order matters. We have multiple threads reading and writing variables using atomics and normal data.
Any clashes should be handled by atomics.
*/
void projectrtpchannel::readsomertp( void )
{
  AQUIRESPINLOCK( this->rtpbufferlock );
  rtppacket* buf = this->inbuff->reserve();
  if( nullptr == buf ) {
    this->inbuff->pop();
    buf = this->inbuff->reserve();
  }
  RELEASESPINLOCK( this->rtpbufferlock );
  if( nullptr == buf ) {
    std::cerr << "ERROR: we should never get here - we have no more buffer available on port " << this->port << std::endl;
    return;
  }

  this->rtpsocket.async_receive_from(
    boost::asio::buffer( buf->pk, RTPMAXLENGTH ), this->rtpsenderendpoint,
      [ this, buf ]( boost::system::error_code ec, std::size_t bytes_recvd ) {
        /* TODO - check for expected size for RTP */
        if ( !ec && bytes_recvd > 0 && bytes_recvd < RTPMAXLENGTH ) {

          /* TODO - pull the DTLS handshake out into its own function */
          /* To be finished */
          if( this->rtpdtlshandshakeing ) {
            this->rtpdtls->write( buf, bytes_recvd );
            auto dtlsstate = this->rtpdtls->handshake();
            if( GNUTLS_E_AGAIN == dtlsstate ) {
              this->readsomertp();
            } else if ( GNUTLS_E_SUCCESS == dtlsstate ) {
              this->readsomertp();
              this->rtpdtlshandshakeing = false;
            }
            return;
          }

          /* We should still count packets we are instructed to drop */
          this->receivedpkcount++;

          if( !this->recv ) {
            if( !ec && bytes_recvd && this->active ) {
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

          buf->length = bytes_recvd;

          AQUIRESPINLOCK( this->rtpbufferlock );
          this->inbuff->push();
          RELEASESPINLOCK( this->rtpbufferlock );
        } else if( !ec ) {
          this->receivedpkcount++;
          this->receivedpkskip++;
        }

        if( !ec && bytes_recvd && this->active ) {
          this->readsomertp();
        }
      } );
}

/*!md
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

/*!md
## handlereadsomertcp
Wait for RTP data
*/
void projectrtpchannel::readsomertcp( void )
{
  this->rtcpsocket.async_receive_from(
  boost::asio::buffer( &this->rtcpdata[ 0 ], RTCPMAXLENGTH ), this->rtcpsenderendpoint,
    [ this ]( boost::system::error_code ec, std::size_t bytes_recvd )
    {
      if ( !ec && bytes_recvd > 0 && bytes_recvd <= RTCPMAXLENGTH )
      {
        this->handlertcpdata();
      }

      if( !ec && bytes_recvd && this->active )
      {
        this->readsomertcp();
      }
    } );
}

/*!md
## isactive
As it says.
*/
bool projectrtpchannel::isactive( void ) {
  return this->active;
}

/*!md
## writepacket
Send a [RTP] packet to our endpoint.
*/
void projectrtpchannel::writepacket( rtppacket *pk ) {

  if( !this->active ) return;

  if( nullptr == pk ) {
    fprintf( stderr, "We have been given an nullptr RTP packet??\n" );
    return;
  }

  if( 0 == pk->length ) {
    fprintf( stderr, "We have been given an RTP packet of zero length??\n" );
    return;
  }

  if( !this->send ) {
    /* silently drop - could we do this sooner to use less CPU? */
    return;
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

  if( e == boost::asio::error::operation_aborted ||
      it == end ) {
    /* Failure - silent (the call will be as well!) */
    fprintf( stderr, "Failed to find target\n" );
    return;
  }

  this->confirmedrtpsenderendpoint = *it;
  this->targetconfirmed = true;

  /* allow us to re-auto correct */
  this->receivedrtp = false;
}

void projectrtpchannel::rfc2833( unsigned short pt ) {
  this->rfc2833pt = pt;
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

    this->mixer->addchannel( shared_from_this() );
    this->mixer->addchannel( other );
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
## unmix
As it says.
*/
void projectrtpchannel::unmix( void ) {
#warning - TODO
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

  chan->requestclose = true;

  return createnapibool( env, true );
}

/* channel at the moment - but future proof to include other stats */
static napi_value stats( napi_env env, napi_callback_info info ) {
  napi_value result, channel;
  if( napi_ok != napi_create_object( env, &result ) ) return NULL;

  if( napi_ok != napi_create_object( env, &channel ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "channel", channel ) ) return NULL;

  napi_value av, chcount;
  if( napi_ok != napi_create_double( env, availableports.size(), &av ) ) return NULL;
  if( napi_ok != napi_create_double( env, channelscreated, &chcount ) ) return NULL;

  if( napi_ok != napi_set_named_property( env, channel, "available", av ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, channel, "current", chcount ) ) return NULL;

  return result;
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

  napi_value result, action;
  if( napi_ok != napi_create_object( env, &result ) ) return NULL;
  if( napi_ok != napi_create_string_utf8( env, "close", NAPI_AUTO_LENGTH, &action ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "action", action ) ) return NULL;

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

  napi_value sentpkcount;
  if( napi_ok != napi_create_double( env, p->outpkcount, &sentpkcount ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, out, "count", sentpkcount ) ) return NULL;

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
    availableports.push( p->getport() );
    napi_release_threadsafe_function( p->cb, napi_tsfn_abort );
  }

  delete ev;
}

static napi_value channelcreate( napi_env env, napi_callback_info info ) {

  size_t argc = 2;
  napi_value argv[ 2 ];
  napi_value ntarget, nport, naddress, ncodec;

  napi_value result;
  auto ourport = availableports.front();
  availableports.pop();

  int32_t targetport;
  char targetaddress[ 128 ];

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, nullptr, nullptr ) ) return NULL;

  if( argc < 1 || argc > 2 ) {
    napi_throw_error( env, "0", "You must provide 1 or 2 params" );
    return NULL;
  }

  if( napi_ok != napi_get_named_property( env, argv[ 0 ], "target", &ntarget ) ) {
    napi_throw_error( env, "0", "Missing target in object" );
    return NULL;
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

  uint32_t codecval = 0;
  napi_get_value_uint32( env, ncodec, &codecval );

  size_t bytescopied;
  napi_get_value_string_utf8( env, naddress, targetaddress, sizeof( targetaddress ), &bytescopied );
  if( 0 == bytescopied || bytescopied >= sizeof( targetaddress ) ) {
    napi_throw_error( env, "1", "Target host address too long" );
    return NULL;
  }

  projectrtpchannel::pointer p = projectrtpchannel::create( ourport );

  /* optional - these have defaults */
  napi_value ndirection;
  bool hasit;
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
    if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, dummyclose, nullptr, &callback ) ) return NULL;
  } else {
    callback = argv[ 1 ];
  }

  napi_value workname;

  if( napi_ok != napi_create_string_utf8( env, "projectrtp", NAPI_AUTO_LENGTH, &workname ) ) {
    return NULL;
  }

  /*
    We don't need to call napi_acquire_threadsafe_function as we are setting
    the inital value of initial_thread_count to 1.
  */
  if( napi_ok != napi_create_threadsafe_function( env,
                                       callback,
                                       NULL,
                                       workname,
                                       0,
                                       1,
                                       NULL,
                                       NULL,
                                       NULL,
                                       eventcallback,
                                       &p->cb ) ) return NULL;

  if( napi_ok != napi_create_object( env, &result ) ) return NULL;
  if( napi_ok != napi_type_tag_object( env, result, &channelcreatetag ) ) return NULL;
  if( napi_ok != napi_wrap( env, result, pb, channeldestroy, pb, nullptr ) ) return NULL;

  p->jsthis = result;
  p->requestopen( targetaddress, targetport, codecval );

  /* methods */
  napi_value mfunc;
  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, channelclose, nullptr, &mfunc ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "close", mfunc ) ) return NULL;

  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, channelmix, nullptr, &mfunc ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "mix", mfunc ) ) return NULL;

  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, channelecho, nullptr, &mfunc ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "echo", mfunc ) ) return NULL;

  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, channelplay, nullptr, &mfunc ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "play", mfunc ) ) return NULL;

  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, channelrecord, nullptr, &mfunc ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "record", mfunc ) ) return NULL;

  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, channeldirection, nullptr, &mfunc ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "direction", mfunc ) ) return NULL;

  /* values */
  napi_value nourport;
  if( napi_ok != napi_create_int32( env, ourport, &nourport ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "port", nourport ) ) return NULL;

  return result;
}

void initrtpchannel( napi_env env, napi_value &result ) {
  napi_value ccreate, cstats;

  for( int i = 10000; i < 20000; i = i + 2 ) {
    availableports.push( i );
  }

  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, channelcreate, nullptr, &ccreate ) ) return;
  if( napi_ok != napi_set_named_property( env, result, "openchannel", ccreate ) ) return;

  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, stats, nullptr, &cstats ) ) return;
  if( napi_ok != napi_set_named_property( env, result, "stats", cstats ) ) return;

}

#endif /* NODE_MODULE */
