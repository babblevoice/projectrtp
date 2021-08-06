

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

/*!md
# Project RTP Channel

This file (class) represents an RP channel. That is an RTP stream (UDP) with
its pair RTCP socket. Basic functions for

1. Opening and closing channels
2. bridging 2 channels
3. Sending data to an endpoint based on us receiving data first or (to be
implimented) the address and port given to us when opening in the channel.

*/
projectrtpchannel::projectrtpchannel( unsigned short port ):
  selectedcodec( 0 ),
  ssrcout( 0 ),
  tsout( 0 ),
  snout( 0 ),
  rtpbufferlock( false ),
  rtpoutindex( 0 ),
  active( false ),
  port( port ),
  rfc2833pt( 0 ),
  lasttelephoneevent( 0 ),
  resolver( workercontext ),
  rtpsocket( workercontext ),
  rtcpsocket( workercontext ),
  receivedrtp( false ),
  targetconfirmed( false ),
  receivedpkcount( 0 ),
  receivedpkskip( 0 ),
  others( nullptr ),
  player( nullptr ),
  newplaydef( nullptr ),
  doecho( false ),
  tick( workercontext ),
  tickswithnortpcount( 0 ),
  send( true ),
  recv( true ),
  havedata( false ),
  newrecorders( MIXQUEUESIZE ),
  maxticktime( 0 ),
  totalticktime( 0 ),
  totaltickcount( 0 ),
  rtpdtls( nullptr ),
  rtpdtlshandshakeing( false ) {
}

void projectrtpchannel::requestopen( void ) {
  boost::asio::post( workercontext,
        boost::bind( &projectrtpchannel::doopen, shared_from_this() ) );
}

void projectrtpchannel::doopen( void ) {

  this->maxticktime = 0;
  this->totalticktime = 0;
  this->totaltickcount = 0;

  this->receivedpkcount = 0;
  this->receivedpkskip = 0;

  this->rtpoutindex = 0;

  this->outcodec.reset();
  this->incodec.reset();

  this->rtpsocket.open( boost::asio::ip::udp::v4() );
  this->rtpsocket.bind( boost::asio::ip::udp::endpoint(
      boost::asio::ip::udp::v4(), this->port ) );

  this->rtcpsocket.open( boost::asio::ip::udp::v4() );
  this->rtcpsocket.bind( boost::asio::ip::udp::endpoint(
      boost::asio::ip::udp::v4(), this->port + 1 ) );

  this->receivedrtp = false;
  this->active = true;
  this->send = true;
  this->recv = true;

  this->codecs.clear();
  this->selectedcodec = 0;

  this->rfc2833pt = 0;
  this->lasttelephoneevent = 0;

  this->ssrcout = rand();

  /* anchor our out time to when the channel is opened */
  this->tsout = std::chrono::system_clock::to_time_t( std::chrono::system_clock::now() );

  this->snout = rand();

  this->tickswithnortpcount = 0;

  this->havedata = false;

  this->readsomertp();
  this->readsomertcp();

  this->tick.expires_after( std::chrono::milliseconds( 20 ) );
  this->tick.async_wait( boost::bind( &projectrtpchannel::handletick,
                                        shared_from_this(),
                                        boost::asio::placeholders::error ) );

}

/*!md
## projectrtpchannel destructor
Clean up
*/
projectrtpchannel::~projectrtpchannel( void ) {
}

/*!md
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

/*!md
## close
Closes the channel.
*/
void projectrtpchannel::requestclose( void ) {
  availableports.push( this->port );
  this->active = false;

  boost::asio::post( workercontext,
        boost::bind( &projectrtpchannel::doclose, this ) );
}

void projectrtpchannel::requestecho( bool e ) {
  this->doecho = e;
}

void projectrtpchannel::doclose( void ) {
  this->active = false;
  this->tick.cancel();
  this->player = nullptr;
  this->newplaydef = nullptr;

  /* remove oursevelse from our list of mixers */
  this->others = nullptr;

  /* close our session if we have one */
  this->rtpdtls = nullptr;
  this->rtpdtlshandshakeing = false;

  /* close up any remaining recorders */
  for( auto& rec: this->recorders ) {
    rec->finishreason = "channel closed";
  }

  this->recorders.clear();

  RELEASESPINLOCK( this->rtpbufferlock );

  this->rtpsocket.close();
  this->rtcpsocket.close();
#warning TODO
#if 0
  if( this->control )
  {
    this->control->channelclosed( this->uuid );

    JSON::Object v;
    v[ "action" ] = "close";
    v[ "id" ] = this->id;
    v[ "uuid" ] = this->uuid;

    /* calculate mos - calc borrowed from FS - thankyou. */
    JSON::Object i;
    if( this->receivedpkcount > 0 )
    {
      double r = ( ( this->receivedpkcount - this->receivedpkskip ) / this->receivedpkcount ) * 100.0;
      if ( r < 0 || r > 100 ) r = 100;
      double mos = 1 + ( 0.035 * r ) + (.000007 * r * ( r - 60 ) * ( 100 - r ) );

      i[ "mos" ] = ( JSON::Double ) mos;
    }
    else
    {
      i[ "mos" ] = ( JSON::Double ) 0.0;
    }
    i[ "count" ] = ( JSON::Integer ) this->receivedpkcount;
    i[ "skip" ] = ( JSON::Integer ) this->receivedpkskip;

    JSON::Object s;
    s[ "in" ] = i;

    if( this->totaltickcount > 0 )
    {
      s[ "meanticktimeus" ] = ( JSON::Integer ) ( this->totalticktime / this->totaltickcount );
    }
    else
    {
      s[ "meanticktimeus" ] = ( JSON::Integer ) 0;
    }
    s[ "maxticktimeus" ] = ( JSON::Integer ) this->maxticktime;
    s[ "tickswithnortpcount" ] = ( JSON::Integer ) this->tickswithnortpcount;
    s[ "totaltickcount" ] = ( JSON::Integer ) this->totaltickcount;

    v[ "stats" ] = s;

    this->control->sendmessage( v );
  }
#endif
}

bool projectrtpchannel::checkidlerecv( void )
{
  if( this->recv && this->active )
  {
    this->tickswithnortpcount++;
    if( this->tickswithnortpcount > ( 50 * 20 ) ) /* 50 (@20mS ptime)/S */
    {
      this->doclose();
      return true;
    }
  }

  return false;
}

/*
Check for new channels to add to the mix in our own thread.
*/
void projectrtpchannel::checkfornewrecorders( void )
{
  boost::shared_ptr< channelrecorder > rec;
  while( this->newrecorders.pop( rec ) )
  {
    rec->sfile = soundfile::create(
        rec->file,
        soundfile::wavformatfrompt( this->selectedcodec ),
        rec->numchannels,
        soundfile::getsampleratefrompt( this->selectedcodec ) );
#if 0
    rec->control = this->control;

    if( this->control )
    {
      JSON::Object v;
      v[ "action" ] = "record";
      v[ "id" ] = this->id;
      v[ "uuid" ] = rec->uuid;
      v[ "chaneluuid" ] = this->uuid;
      v[ "file" ] = rec->file;
      v[ "state" ] = "recording";

      this->control->sendmessage( v );
    }
#endif

    this->recorders.push_back( rec );
  }
}

/*!md
## handletick
Our timer to send data - use this for when we are a single channel. Mixing tick is done in mux.
*/
void projectrtpchannel::handletick( const boost::system::error_code& error )
{
  if ( error != boost::asio::error::operation_aborted )
  {
    /* calc a timer */
    boost::posix_time::ptime nowtime( boost::posix_time::microsec_clock::local_time() );

    /* only us */
    projectchannelmux::pointer mux = this->others;
    if( nullptr == mux || 0 == mux->size() )
    {
      if( this->checkidlerecv() ) return;
      this->checkfornewrecorders();

      AQUIRESPINLOCK( this->rtpbufferlock );
      rtppacket *src = this->inbuff->pop();
      RELEASESPINLOCK( this->rtpbufferlock );

      if( nullptr != src )
      {
        this->incodec << codecx::next;
        this->incodec << *src;
      }

      stringptr newplaydef = this->newplaydef.load();
      if( newplaydef )
      {
#warning remove dependanciy on c++ json - node can handle this now
        if( !this->player )
        {
          this->player = soundsoup::create();
        }

        JSON::Value ob = JSON::parse( *newplaydef );
        this->player->config( JSON::as_object( ob ), selectedcodec );

        this->newplaydef = nullptr;
      }
      else if( this->player )
      {
        rtppacket *out = this->gettempoutbuf();
        rawsound r;
        if( this->player->read( r ) && r.size() > 0 )
        {
          this->outcodec << codecx::next;
          this->outcodec << r;
          out << this->outcodec;
          this->writepacket( out );
        }
      }
      else if( this->doecho )
      {
        if( nullptr != src )
        {
          this->outcodec << codecx::next;
          this->outcodec << *src;

          rtppacket *dst = this->gettempoutbuf();
          dst << this->outcodec;
          this->writepacket( dst );
        }
      }

      this->writerecordings();
    }

    boost::posix_time::time_duration const diff = ( boost::posix_time::microsec_clock::local_time() - nowtime );
    uint64_t tms = diff.total_microseconds();
    this->totalticktime += tms;
    this->totaltickcount++;
    if( tms > this->maxticktime ) this->maxticktime = tms;

    /* The last thing we do */
    this->tick.expires_at( this->tick.expiry() + boost::asio::chrono::milliseconds( 20 ) );
    this->tick.async_wait( boost::bind( &projectrtpchannel::handletick,
                                        shared_from_this(),
                                        boost::asio::placeholders::error ) );
  }
}

/*
# writerecordings
If our codecs (in and out) have data then write to recorded files.
*/

static bool recorderfinished( boost::shared_ptr<channelrecorder> rec )
{
  boost::posix_time::ptime nowtime( boost::posix_time::microsec_clock::local_time() );
  boost::posix_time::time_duration const diff = ( nowtime - rec->created );

  if( diff.total_milliseconds() < rec->minduration  )
  {
    return false;
  }

  if( rec->active && rec->lastpowercalc < rec->finishbelowpower )
  {
    rec->finishreason = "finishbelowpower";
    return true;
  }

  //std::cout << diff.total_milliseconds() << ":" << rec->maxduration << std::endl;
  if( diff.total_milliseconds() > rec->maxduration )
  {
    rec->finishreason = "timeout";
    return true;
  }

  return false;
}

void projectrtpchannel::writerecordings( void )
{
  if( 0 == this->recorders.size() ) return;

  uint16_t power = 0;

  /* Decide if we need to calc power as it is expensive */
  for( auto& rec: this->recorders )
  {
    if( ( !rec->active && 0 != rec->startabovepower ) || 0 != rec->finishbelowpower )
    {
      power = this->incodec.power();
      break;
    }
  }

  for( auto& rec: this->recorders )
  {
    auto pav = rec->poweravg( power );
    if( 0 == rec->startabovepower && 0 == rec->finishbelowpower )
    {
      rec->active = true;
    }
    else
    {
      if( !rec->active && pav > rec->startabovepower )
      {
        rec->active = true;
      }
    }

    if( rec->active )
    {
      rec->sfile->write( this->incodec, this->outcodec );
    }
  }

  this->recorders.remove_if( recorderfinished );
}

/*!md
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
        /* To be finished */
        if ( !ec && bytes_recvd > 0 && bytes_recvd <= RTPMAXLENGTH ) {
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
          } else {
            /* After the first packet - we only accept data from the verified source */
            if( this->confirmedrtpsenderendpoint != this->rtpsenderendpoint ) {
              this->readsomertp();
              return;
            }
          }

          buf->length = bytes_recvd;

          AQUIRESPINLOCK( this->rtpbufferlock );
          this->inbuff->push();
          RELEASESPINLOCK( this->rtpbufferlock );
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
rtppacket *projectrtpchannel::gettempoutbuf( uint32_t skipcount ) {
  int outindex = this->rtpoutindex++;
  rtppacket *buf = &this->outrtpdata[ outindex % BUFFERPACKETCOUNT ];

  buf->init( this->ssrcout );
  buf->setpayloadtype( this->selectedcodec );

  this->snout += skipcount;
  buf->setsequencenumber( this->snout );

  if( skipcount > 0 ) {
    this->tsout += ( buf->getticksperpacket() * skipcount );
  }

  buf->settimestamp( this->tsout );
  this->snout++;

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
bool projectrtpchannel::isactive( void )
{
  return this->active;
}

/*!md
## writepacket
Send a [RTP] packet to our endpoint.
*/
void projectrtpchannel::writepacket( rtppacket *pk )
{
  if( 0 == pk->length )
  {
    std::cerr << "We have been given an RTP packet of zero length??" << std::endl;
    return;
  }

  if( !this->send )
  {
    /* silently drop - could we do this sooner to use less CPU? */
    return;
  }

  if( this->receivedrtp || this->targetconfirmed )
  {
    this->tsout = pk->getnexttimestamp();

    this->rtpsocket.async_send_to(
                      boost::asio::buffer( pk->pk, pk->length ),
                      this->confirmedrtpsenderendpoint,
                      boost::bind( &projectrtpchannel::handlesend,
                                    this,
                                    boost::asio::placeholders::error,
                                    boost::asio::placeholders::bytes_transferred ) );
  }
}

/*!md
## target
Our control can set the target of the RTP stream. This can be important in order to open holes in firewall for our reverse traffic.
*/
void projectrtpchannel::target( std::string &address, unsigned short port ) {
  this->receivedrtp = false;
  boost::asio::ip::udp::resolver::query query( boost::asio::ip::udp::v4(), address, std::to_string( port ) );

  /* Resolve the address */
  this->resolver.async_resolve( query,
      boost::bind( &projectrtpchannel::handletargetresolve,
        shared_from_this(),
        boost::asio::placeholders::error,
        boost::asio::placeholders::iterator ) );
}

void projectrtpchannel::rfc2833( unsigned short pt ) {
  this->rfc2833pt = pt;
}

/*!md
## mix
Add the other to our list of others. n way relationship. Adds to queue for when our main thread calls into us.
*/
bool projectrtpchannel::mix( projectrtpchannel::pointer other ) {
  projectrtpchannel::pointer tmpother;
  if( this == other.get() ) {
    return true;
  }

  projectchannelmux::pointer m = this->others.load( std::memory_order_relaxed );
  if( nullptr == m ) {
    m = projectchannelmux::create( workercontext );
    m->addchannel( shared_from_this() );
    m->addchannel( other );
    /* We don't need our channel timer */
    m->go();
  } else {
    m->addchannel( other );
  }

  return true;
}

/*!md
## unmix
As it says.
*/
void projectrtpchannel::unmix( void ) {
  this->others = nullptr;
}

/*!md
## audio
The CODECs on the other end which are acceptable. The first one should be the preferred. For now we keep hold of the list of codecs as we may be using them in the future. Filter out non-RTP streams (such as DTMF).
*/
bool projectrtpchannel::audio( codeclist codecs ) {
  this->codecs = codecs;
  codeclist::iterator it;
  for( it = codecs.begin(); it != codecs.end(); it++ ) {
    switch( *it ) {
      case PCMAPAYLOADTYPE:
      case PCMUPAYLOADTYPE:
      case G722PAYLOADTYPE:
      case ILBCPAYLOADTYPE: {
        this->selectedcodec = *it;
        return true;
      }
    }
  }
  return false;
}

/*!md
## handletargetresolve
We have resolved the target address and port now use it. Further work could be to inform control there is an issue.
*/
void projectrtpchannel::handletargetresolve (
            boost::system::error_code e,
            boost::asio::ip::udp::resolver::iterator it ) {
  boost::asio::ip::udp::resolver::iterator end;

  if( it == end )
  {
    /* Failure - silent (the call will be as well!) */
    return;
  }

  this->confirmedrtpsenderendpoint = *it;
  this->targetconfirmed = true;

  /* allow us to re-auto correct */
  this->receivedrtp = false;
}

/*!md
## handlesend
What is called once we have sent something.
*/
void projectrtpchannel::handlesend(
      const boost::system::error_code& error,
      std::size_t bytes_transferred) {

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

static napi_value channelclose( napi_env env, napi_callback_info info ) {

  projectrtpchannel::pointer chan = getrtpchannelfromthis( env, info );
  if( nullptr == chan ) createnapibool( env, false );

  chan->requestclose();

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

void channeldestroy( napi_env env, void* instance, void* /* hint */ ) {
  delete ( ( hiddensharedptr * ) instance );
}

static napi_value channelcreate( napi_env env, napi_callback_info info ) {

  size_t argc = 1;
  napi_value argv[ 1 ];


  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, nullptr, nullptr ) ) return NULL;

  napi_value result;
  auto port = availableports.front();
  availableports.pop();

  hiddensharedptr *pb = new hiddensharedptr( projectrtpchannel::create( port ) );

  pb->get< projectrtpchannel >()->requestopen();

  if( napi_ok != napi_create_object( env, &result ) ) return NULL;
  if( napi_ok != napi_type_tag_object( env, result, &channelcreatetag ) ) return NULL;
  if( napi_ok != napi_wrap( env, result, pb, channeldestroy, nullptr, nullptr ) ) return NULL;

  /* methods */
  napi_value mclose, mecho;
  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, channelclose, nullptr, &mclose ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "close", mclose ) ) return NULL;

  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, channelecho, nullptr, &mecho ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "echo", mecho ) ) return NULL;

  /* values */
  napi_value nport;
  if( napi_ok != napi_create_int32( env, port, &nport ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "port", nport ) ) return NULL;

  return result;
}

void initrtpchannel( napi_env env, napi_value &result ) {
  napi_value rtpchan;
  napi_value ccreate;

  for( int i = 10000; i < 20000; i = i + 2 ) {
    availableports.push( i );
  }

  if( napi_ok != napi_create_object( env, &rtpchan ) ) return;
  if( napi_ok != napi_set_named_property( env, result, "rtpchannel", rtpchan ) ) return;
  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, channelcreate, nullptr, &ccreate ) ) return;
  if( napi_ok != napi_set_named_property( env, rtpchan, "create", ccreate ) ) return;

}

#endif /* NODE_MODULE */
