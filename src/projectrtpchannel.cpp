

#include <iostream>
#include <cstdlib>

#include <boost/bind/bind.hpp>
#include <boost/chrono.hpp>
#include <iomanip>
#include <utility>

#include "projectrtpchannel.h"
#include "controlclient.h"

/*
# channelrecorder
c'stor and create

Track files we are recording to.
*/
channelrecorder::channelrecorder( std::string &file ) :
  file( file ),
  poweraverageduration( 1 ),
  startabovepower( 0 ),
  finishbelowpower( 0 ),
  minduration( 0 ),
  maxduration( 0 ),
  numchannels( 2 ),
  active( false ),
  lastpowercalc( 0 ),
  created( boost::posix_time::microsec_clock::local_time() ),
  control( nullptr )
{
}

channelrecorder::~channelrecorder()
{
  if( nullptr != this->control )
  {
    JSON::Object v;
    v[ "action" ] = "record";
    v[ "uuid" ] = this->uuid;
    v[ "state" ] = "finished";
    v[ "reason" ] = finishreason;

    this->control->sendmessage( v );
  }
}

uint16_t channelrecorder::poweravg( uint16_t power )
{
  if( this->poweraverageduration != this->powerfilter.getlength() )
  {
    this->powerfilter.reset( this->poweraverageduration );
  }
  this->lastpowercalc = this->powerfilter.execute( power );
  return this->lastpowercalc;
}

channelrecorder::pointer channelrecorder::create( std::string &file )
{
  return pointer( new channelrecorder( file ) );
}

/*
## c'stor && create
*/
projectchannelmux::projectchannelmux( boost::asio::io_context &iocontext ):
  iocontext( iocontext ),
  tick( iocontext ),
  newchannels( MIXQUEUESIZE ),
  failcount( 0 )
{
}

projectchannelmux::~projectchannelmux()
{
  this->tick.cancel();
}

projectchannelmux::pointer projectchannelmux::create( boost::asio::io_context &iocontext )
{
  return pointer( new projectchannelmux( iocontext ) );
}

void projectchannelmux::mixall( void )
{
  /* First decide on a common rate (if we only have 8K channels it is pointless
  upsampling them all and wasting resources) */
  int l16krequired = L168KPAYLOADTYPE;
  size_t insize = L16PAYLOADSAMPLES;

  for( auto& chan: this->channels )
  {
    switch( chan->selectedcodec )
    {
      case G722PAYLOADTYPE:
      case L1616KPAYLOADTYPE:
      {
        l16krequired = L1616KPAYLOADTYPE;
        goto endofforloop;
      }
    }
  }
  endofforloop:

  this->added.malloc( insize, sizeof( int16_t ), l16krequired );
  this->subtracted.malloc( insize, sizeof( int16_t ), l16krequired );
  this->added.zero();

  /* We first have to add them all up */
  for( auto& chan: this->channels )
  {
    rtppacket *src = chan->getrtpbottom();
    if( nullptr != src )
    {
      chan->incodec << codecx::next;
      chan->incodec << *src;
      this->added += chan->incodec;
    }
  }

  /* Now we subtract this channel to send to this channel. */
  for( auto& chan: this->channels )
  {
    /*
     There is a small chance that rtp bottom may have flipped from nullptr to something.
     We will get a little noise as a result. We could get rid of this by marking the
     channel somehow?
    */
    rtppacket *src = chan->getrtpbottom();
    if( nullptr != src )
    {
      rtppacket *dst = chan->gettempoutbuf();

      this->subtracted.zero();
      this->subtracted.copy( this->added );
      this->subtracted -= chan->incodec;

      chan->outcodec << codecx::next;
      chan->outcodec << this->subtracted;
      dst << chan->outcodec;
      chan->writepacket( dst );
      chan->incrrtpbottom( src );
    }
  }
}

/*
## mix2
More effient mixer for 2 channels where no recording in mono (i.e. they are added) is required.
The caller has to ensure there are 2 channels.
*/
void projectchannelmux::mix2( void )
{
  auto chans = this->channels.begin();
  auto chan1 = *chans++;
  auto chan2 = *chans;

  rtppacket *src;
  if( ( src = chan1->getrtpbottom() ) != nullptr )
  {
    uint16_t workingonaheadby = src->getsequencenumber() - chan1->lastworkedonsn - 1;

    chan1->receivedpkskip += workingonaheadby;
    this->checkfordtmf( chan1, src );

    this->postrtpdata( chan1, chan2, src, workingonaheadby );
    chan1->incrrtpbottom( src );
  }

  if( ( src = chan2->getrtpbottom() ) != nullptr )
  {
    uint16_t workingonaheadby = src->getsequencenumber() - chan2->lastworkedonsn - 1;

    chan2->receivedpkskip += workingonaheadby;
    this->checkfordtmf( chan2, src );

    this->postrtpdata( chan2, chan1, src, workingonaheadby );
    chan2->incrrtpbottom( src );
  }
}

/*
Our timer handler.
*/
void projectchannelmux::handletick( const boost::system::error_code& error )
{
  if ( error != boost::asio::error::operation_aborted )
  {
    boost::posix_time::ptime nowtime( boost::posix_time::microsec_clock::local_time() );

    this->checkfornewmixes();

    for( auto& chan: this->channels )
    {
      chan->checkfornewrecorders();
    }

    /* Check for channels which have request removal */
    {
      bool anyremoved = false;
      repeatremove:
      for( auto& chan: this->channels )
      {
        projectchannelmux::pointer tmp = chan->others.load( std::memory_order_relaxed );
        if( nullptr == tmp )
        {
          this->channels.remove( chan );
          anyremoved = true;
          goto repeatremove;
        }
      }

      if( anyremoved )
      {
        if( this->channels.size() <= 1 )
        {
          if( 1 == this->channels.size() )
          {
            auto chan = this->channels.begin();
            this->channels.erase( chan );
          }
          /* As we use auto pointers returning from this function without
          readding a new pointer to a timer will clean things up */
          return;
        }
      }
    }

    if( 2 == this->channels.size() )
    {
      this->mix2();
    }
    else if( this->channels.size() > 2 )
    {
      this->mixall();
    }

    for( auto& chan: this->channels )
    {
      chan->checkidlerecv();
      chan->checkandfixoverrun();
    }

    /* calc our timer */
    boost::posix_time::time_duration const diff = ( boost::posix_time::microsec_clock::local_time() - nowtime );
    uint64_t tms = diff.total_microseconds();
    for( auto& chan: this->channels )
    {
      chan->totalticktime += tms;
      chan->totaltickcount++;
      if( tms > chan->maxticktime ) chan->maxticktime = tms;
    }


    this->tick.expires_at( this->tick.expiry() + boost::asio::chrono::milliseconds( 20 ) );
    this->tick.async_wait( boost::bind( &projectchannelmux::handletick,
                                        shared_from_this(),
                                        boost::asio::placeholders::error ) );
  }
}

void projectchannelmux::go( void )
{

  this->tick.expires_after( std::chrono::milliseconds( 20 ) );
  this->tick.async_wait( boost::bind( &projectchannelmux::handletick,
                                        shared_from_this(),
                                        boost::asio::placeholders::error ) );
}

/*
Check for new channels to add to the mix in our own thread.
*/
void projectchannelmux::checkfornewmixes( void )
{
  std::shared_ptr< projectrtpchannel > chan;
  while( this->newchannels.pop( chan ) )
  {
    for( auto& checkchan: this->channels )
    {
      if( checkchan == chan ) goto contin;
    }

    this->channels.push_back( chan );
    this->channels.unique();
    chan->others.exchange( shared_from_this() );

    contin:;
  }
}

void projectchannelmux::addchannel( std::shared_ptr< projectrtpchannel > chan )
{
  this->newchannels.push( chan );
}

/*
## checkfordtmf
*/
void projectchannelmux::checkfordtmf( std::shared_ptr< projectrtpchannel > chan, rtppacket *src )
{
  /* The next section is sending to our recipient(s) */
  if( 0 != chan->rfc2833pt && src->getpayloadtype() == chan->rfc2833pt )
  {
    /* We have to look for DTMF events handling issues like missing events - such as the marker or end bit */
    uint16_t sn = src->getsequencenumber();
    uint8_t event = 0;
    uint8_t endbit = 0;

    /*
    there really should be a packet - we should cater for multiple?
    endbits can appear to be sent multiple times.
    */
    if( src->getpayloadlength() >= 4 )
    {
      uint8_t * pl = src->getpayload();
      endbit = pl[ 1 ] >> 7;
      event = pl[ 0 ];
    }

    uint8_t pm = src->getpacketmarker();
    if( !pm && 0 != chan->lasttelephoneevent && abs( static_cast< long long int >( sn - chan->lasttelephoneevent ) ) > 20 )
    {
      pm = 1;
    }

    if( pm )
    {
      if( chan->control )
      {
        JSON::Object v;
        v[ "action" ] = "telephone-event";
        v[ "id" ] = chan->id;
        v[ "uuid" ] = chan->uuid;
        v[ "event" ] = ( JSON::Integer )event;

        chan->control->sendmessage( v );
      }
    }

    if( endbit )
    {
      chan->lasttelephoneevent = 0;
    }
    else
    {
      chan->lasttelephoneevent = sn;
    }
  }
}

/*
## postrtpdata
Send the data somewhere.
*/
void projectchannelmux::postrtpdata( std::shared_ptr< projectrtpchannel > srcchan,  std::shared_ptr< projectrtpchannel > dstchan, rtppacket *src, uint32_t skipcount )
{
  rtppacket *dst = dstchan->gettempoutbuf( skipcount );

  if( nullptr == dst )
  {
    std::cerr << "We have a null out buffer" << std::endl;
    return;
  }

  /* This needs testing */
  if( 0 != srcchan->rfc2833pt && src->getpayloadtype() == srcchan->rfc2833pt )
  {
    dst->setpayloadtype( srcchan->rfc2833pt );
    dst->copy( src );
  }
  else
  {
    dstchan->outcodec << codecx::next;
    dstchan->outcodec << *src;
    dst << dstchan->outcodec;
  }

  dstchan->writepacket( dst );
}

/*!md
# Project RTP Channel

This file (class) represents an RP channel. That is an RTP stream (UDP) with its pair RTCP socket. Basic functions for

1. Opening and closing channels
2. bridging 2 channels
3. Sending data to an endpoint based on us receiving data first or (to be implimented) the address and port given to us when opening in the channel.


## projectrtpchannel constructor
Create the socket then wait for data

echo "This is my data" > /dev/udp/127.0.0.1/10000
*/
projectrtpchannel::projectrtpchannel( boost::asio::io_context &iocontext, unsigned short port )
  :
  selectedcodec( 0 ),
  ssrcout( 0 ),
  ssrcin( 0 ),
  tsout( 0 ),
  seqout( 0 ),
  toolatertppacket( nullptr ),
  orderedinminsn( 0 ),
  orderedinmaxsn( 0 ),
  lastworkedonsn( 0 ),
  rtpbuffercount( BUFFERPACKETCOUNT ),
  rtpbufferlock( false ),
  rtpoutindex( 0 ),
  active( false ),
  port( port ),
  rfc2833pt( 0 ),
  lasttelephoneevent( 0 ),
  iocontext( iocontext ),
  resolver( iocontext ),
  rtpsocket( iocontext ),
  rtcpsocket( iocontext ),
  receivedrtp( false ),
  targetconfirmed( false ),
  reader( true ),
  writer( true ),
  receivedpkcount( 0 ),
  receivedpkskip( 0 ),
  others( nullptr ),
  player( nullptr ),
  newplaydef( nullptr ),
  doecho( false ),
  tick( iocontext ),
  tickswithnortpcount( 0 ),
  send( true ),
  recv( true ),
  havedata( false ),
  newrecorders( MIXQUEUESIZE ),
  maxticktime( 0 ),
  totalticktime( 0 ),
  totaltickcount( 0 )
{
  for( auto i = 0; i < BUFFERPACKETCOUNT; i ++ )
  {
    this->orderedrtpdata[ i ] = nullptr;
    this->availablertpdata[ i ] = &this->rtpdata[ i ];
  }
}

/*!md
## projectrtpchannel destructor
Clean up
*/
projectrtpchannel::~projectrtpchannel( void )
{
  this->player = nullptr;
  this->others = nullptr;
}

/*!md
# create

*/
projectrtpchannel::pointer projectrtpchannel::create( boost::asio::io_context &iocontext, unsigned short port )
{
  return pointer( new projectrtpchannel( iocontext, port ) );
}

/*!md
## open
Open the channel to read network data. Setup memory and pointers.
*/
void projectrtpchannel::open( std::string &id, std::string &uuid, controlclient::pointer c )
{
  this->id = id;
  this->uuid = uuid;
  this->control = c;

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
  this->toolatertppacket = nullptr;
  this->active = true;
  this->send = true;
  this->recv = true;

  this->codecs.clear();
  this->selectedcodec = 0;

  this->rfc2833pt = 0;
  this->lasttelephoneevent = 0;

  this->readsomertp();
  this->readsomertcp();

  this->ssrcout = rand();

  /* anchor our out time to when the channel is opened */
  this->tsout = std::chrono::system_clock::to_time_t( std::chrono::system_clock::now() );

  this->seqout = 0;

  this->orderedinminsn = 0;
  this->orderedinmaxsn = 0;
  this->lastworkedonsn = 0;

  this->tickswithnortpcount = 0;

  this->havedata = false;

  this->go();
}

void projectrtpchannel::go( void )
{
  this->tick.expires_after( std::chrono::milliseconds( 20 ) );
  this->tick.async_wait( boost::bind( &projectrtpchannel::handletick,
                                        shared_from_this(),
                                        boost::asio::placeholders::error ) );
}

unsigned short projectrtpchannel::getport( void )
{
  return this->port;
}

/*!md
## close
Closes the channel.
*/
void projectrtpchannel::close( void )
{
  this->active = false;
  boost::asio::post( this->iocontext,
        boost::bind( &projectrtpchannel::doclose, this ) );
}

void projectrtpchannel::doclose( void )
{
  this->active = false;
  this->tick.cancel();
  this->player = nullptr;
  this->newplaydef = nullptr;

  /* remove oursevelse from our list of mixers */
  this->others = nullptr;

  for( auto i = 0; i < BUFFERPACKETCOUNT; i ++ )
  {
    this->orderedrtpdata[ i ] = nullptr;
    this->availablertpdata[ i ] = &this->rtpdata[ i ];
  }

  /* close up any remaining recorders */
  for( auto& rec: this->recorders )
  {
    rec->finishreason = "channel closed";
  }

  this->recorders.clear();

  this->rtpbuffercount = BUFFERPACKETCOUNT;
  this->rtpbufferlock.store( false, std::memory_order_release );

  this->rtpsocket.close();
  this->rtcpsocket.close();

  if( this->control )
  {
    this->control->channelclosed( this->uuid );

    JSON::Object v;
    v[ "action" ] = "close";
    v[ "id" ] = this->id;
    v[ "uuid" ] = this->uuid;

    /* calculate mos - calc borrowed from FS - thankyou. */
    if( this->receivedpkcount > 0 )
    {
      double r = ( ( this->receivedpkcount - this->receivedpkskip ) / this->receivedpkcount ) * 100.0;
      if ( r < 0 || r > 100 ) r = 100;
      double mos = 1 + ( 0.035 * r ) + (.000007 * r * ( r - 60 ) * ( 100 - r ) );

      JSON::Object i;
      i[ "mos" ] = ( JSON::Double ) mos;
      i[ "count" ] = ( JSON::Integer ) this->receivedpkcount;
      i[ "skip" ] = ( JSON::Integer ) this->receivedpkskip;

      JSON::Object s;
      s[ "in" ] = i;

      if( this->totaltickcount > 0 )
      {
        s[ "maxticktimeus" ] = ( JSON::Integer ) this->maxticktime;
        s[ "meanticktimeus" ] = ( JSON::Integer ) ( this->totalticktime / this->totaltickcount );
      }

      v[ "stats" ] = s;
    }

    this->control->sendmessage( v );
  }
}

bool projectrtpchannel::checkidlerecv( void )
{
  if( this->recv && this->active )
  {
    this->tickswithnortpcount++;
    if( this->tickswithnortpcount > 400 )
    {
      this->close();
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

    rec->control = this->control;
    this->recorders.push_back( rec );

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
      this->checkandfixoverrun();
      this->checkfornewrecorders();

      rtppacket *src = this->getrtpbottom();
      if( nullptr != src )
      {
        this->incodec << codecx::next;
        this->incodec << *src;
        this->incrrtpbottom( src );
      }

      stringptr newplaydef = this->newplaydef.load();
      if( newplaydef )
      {
        try
        {
          if( !this->player )
          {
            this->player = soundsoup::create();
          }

          JSON::Value ob = JSON::parse( *newplaydef );
          this->player->config( JSON::as_object( ob ), selectedcodec );
        }
        catch(...)
        {
          std::cerr << "Bad sound soup: " << *newplaydef << std::endl;
        }
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
        this->outcodec << codecx::next;
        this->outcodec << *src;

        rtppacket *dst = this->gettempoutbuf();
        dst << this->outcodec;
        this->writepacket( dst );
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

/*
## getrtpbottom
Gets our oldest RTP packet required processing.highcount and lowcount
provide some hysteresis so we don't quickly stop and start a channel
only really needed if we need to mix from multiple sources (i.e. conference).
*/
rtppacket *projectrtpchannel::getrtpbottom( uint16_t highcount, uint16_t lowcount )
{
  if( !this->receivedrtp ) return nullptr;

  /* There probably has been a network delay and they are now coming through
  which has filled up our RTP buffer - help clear out */
  uint16_t diff = this->orderedinmaxsn - this->orderedinminsn;
  if( diff > BUFFERPACKETCAP )
  {
    for( uint16_t i = 0; i < diff; i++ )
    {
      this->incrrtpbottom( this->orderedrtpdata[ this->orderedinminsn % BUFFERPACKETCOUNT ] );
    }
  }

  rtppacket *src = this->orderedrtpdata[ this->orderedinminsn % BUFFERPACKETCOUNT ];

  if( nullptr == src ) return nullptr;
  auto sn = src->getsequencenumber();
  if( this->orderedinminsn != sn ) return nullptr;

  auto aheadby = this->orderedinmaxsn - sn;
  auto delaycount = highcount;
  if( this->havedata ) delaycount = lowcount;

  /* We allow n to accumulate in our buffer before we work on them */
  if( aheadby < delaycount )
  {
    this->havedata = false;
    return nullptr;
  }
  this->havedata = true;

  return src;
}

void projectrtpchannel::incrrtpbottom( rtppacket *from )
{
  if( nullptr == from ) return;
  this->lastworkedonsn = from->getsequencenumber();
  this->returnbuffer( this->orderedrtpdata[ this->lastworkedonsn % BUFFERPACKETCOUNT ] );
  this->orderedrtpdata[ this->lastworkedonsn % BUFFERPACKETCOUNT ] = nullptr;
  this->orderedinminsn++;
}

/*
Get and return an avaiable memory buffer for an rtp packet. Use a spin lock for a tiny section.
*/
void projectrtpchannel::returnbuffer( rtppacket *buf )
{
  while( this->rtpbufferlock.exchange( true, std::memory_order_acquire ) );

  this->availablertpdata[ this->rtpbuffercount ] = buf;
  this->rtpbuffercount++;

  this->rtpbufferlock.store( false, std::memory_order_release );
}


rtppacket* projectrtpchannel::getbuffer( void )
{
  rtppacket* buf = nullptr;
  while( this->rtpbufferlock.exchange( true, std::memory_order_acquire ) );

  if( this->rtpbuffercount == 0 )
  {
    goto getbufferend;
  }

  {
    auto currentmax = --this->rtpbuffercount;

    buf = this->availablertpdata[ currentmax ];
    this->availablertpdata[ currentmax ] = nullptr;
  }

  getbufferend:
  this->rtpbufferlock.store( false, std::memory_order_release );
  return buf;
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
  rtppacket *buf = this->getbuffer();
  if( nullptr == buf ) return;

  this->rtpsocket.async_receive_from(
    boost::asio::buffer( buf->pk, RTPMAXLENGTH ),
                          this->rtpsenderendpoint,
      [ this, buf ]( boost::system::error_code ec, std::size_t bytes_recvd )
      {
        if ( !ec && bytes_recvd > 0 && bytes_recvd <= RTPMAXLENGTH )
        {
          if( !this->recv )
          {
            /* silently drop */
            this->returnbuffer( buf );
            if( !ec && bytes_recvd && this->active )
            {
              this->readsomertp();
            }
            return;
          }

          /* As we check if there is any buffer left this should stop us ever running out and quitting trying
          i.e. when we are very low imm return and retry - eventually our tick will process the backlog */
          if( 0 == this->rtpbuffercount )
          {
            /* silently drop this packet and repeat until we have space in our buffer  */
            std::cerr << "Dropping packet due to low space in buffer" << std::endl;
            this->returnbuffer( buf );
            this->readsomertp();
            return;
          }

#ifdef SIMULATEDPACKETLOSSRATE
          /* simulate packet loss */
          if( 0 == rand() % SIMULATEDPACKETLOSSRATE )
          {
            this->returnbuffer( buf );
            if( !ec && bytes_recvd && this->active )
            {
              this->readsomertp();
            }
            return;
          }
#endif
          this->tickswithnortpcount = 0;
          this->receivedpkcount++;
          if( !this->receivedrtp )
          {
            this->lastworkedonsn.store( buf->getsequencenumber() - 1 ); /* pseudo */
            this->orderedinminsn.store( buf->getsequencenumber() );
            this->orderedinmaxsn.store( buf->getsequencenumber() );
            this->confirmedrtpsenderendpoint = this->rtpsenderendpoint;
            this->receivedrtp = true;
          }
          else
          {
            /* After the first packet - we only accept data from the verified source */
            if( this->confirmedrtpsenderendpoint != this->rtpsenderendpoint )
            {
              this->returnbuffer( buf );
              this->readsomertp();
              return;
            }

            if( this->checkforoverrun( buf ) ) return;
            if( this->checkforunderrun( buf ) ) return;
          }

          /* store in the buffer */
          buf->length = bytes_recvd;

          /* Now order it */
          uint16_t sn = buf->getsequencenumber();
          this->orderedrtpdata[ sn % BUFFERPACKETCOUNT ].store( buf, std::memory_order_relaxed );
          if( sn > this->orderedinmaxsn ) this->orderedinmaxsn = sn;
          if( sn < this->orderedinminsn ) this->orderedinminsn = sn;
        }

        if( !ec && bytes_recvd && this->active )
        {
          this->readsomertp();
        }
      } );
}

/*
## checkforoverrun
See if the received packet is within our window. If it does we have to trash
existing items in our buffer so we have to hand this off to our tick.
*/
bool projectrtpchannel::checkforoverrun( rtppacket *buf )
{
  uint16_t diff = buf->getsequencenumber() - this->orderedinminsn;

  if( diff > BUFFERPACKETCOUNT )
  {
    /* We have to clear the buffer and re-go */
    this->toolatertppacket = buf;
    return true;
  }

  return false;
}

/*
## fixoverrun
When we have received a packet which is too new - we need to clear our buffers
and restart. This MUST be called from our tick.
*/
void projectrtpchannel::checkandfixoverrun( void )
{
  rtppacket *tmp = this->toolatertppacket.load( std::memory_order_relaxed );
  if( nullptr != tmp )
  {
    for( size_t i = 0; i < BUFFERPACKETCOUNT; i++ )
    {
      if( nullptr != this->orderedrtpdata[ i ] )
      {
        this->returnbuffer( this->orderedrtpdata[ i ] );
        this->orderedrtpdata[ i ] = nullptr;
      }
    }

    uint16_t sn = tmp->getsequencenumber();
    this->orderedrtpdata[ sn % BUFFERPACKETCOUNT ].store( tmp, std::memory_order_relaxed );
    this->orderedinminsn = this->orderedinmaxsn = sn;
    this->toolatertppacket = nullptr;

    if( this->active )
    {
      this->readsomertp();
    }
  }
}

/*
## checkforunderrun
Received too late - do we just bin...
*/
bool projectrtpchannel::checkforunderrun( rtppacket *buf )
{
  uint16_t sn = buf->getsequencenumber();

  auto diff = this->orderedinminsn - sn;
  if( diff > 0 && diff < BUFFERPACKETCOUNT  )
  {
    this->returnbuffer( buf );
    if( this->active )
    {
      this->readsomertp();
    }
    return true;
  }
  return false;
}

/*!md
## gettempoutbuf
When we need a buffer to send data out (because we cannot guarantee our own buffer will be available) we can use the circular out buffer on this channel. This will return the next one available.

We assume this is called to send packets out in order, and at intervals required for each timestamp to be incremented in lou of it payload type.
*/
rtppacket *projectrtpchannel::gettempoutbuf( uint32_t skipcount )
{
  rtppacket *buf = &this->outrtpdata[ this->rtpoutindex ];
  this->rtpoutindex = ( this->rtpoutindex + 1 ) % BUFFERPACKETCOUNT;

  buf->init( this->ssrcout );
  buf->setpayloadtype( this->selectedcodec );

  this->seqout += skipcount;
  buf->setsequencenumber( this->seqout );

  if( skipcount > 0 )
  {
    this->tsout += ( buf->getticksperpacket() * skipcount );
  }

  buf->settimestamp( this->tsout );

  this->seqout++;

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
void projectrtpchannel::target( std::string &address, unsigned short port )
{
  this->receivedrtp = false;
  boost::asio::ip::udp::resolver::query query( boost::asio::ip::udp::v4(), address, std::to_string( port ) );

  /* Resolve the address */
  this->resolver.async_resolve( query,
      boost::bind( &projectrtpchannel::handletargetresolve,
        shared_from_this(),
        boost::asio::placeholders::error,
        boost::asio::placeholders::iterator ) );
}

void projectrtpchannel::rfc2833( unsigned short pt )
{
  this->rfc2833pt = pt;
}

/*!md
## mix
Add the other to our list of others. n way relationship. Adds to queue for when our main thread calls into us.
*/
bool projectrtpchannel::mix( projectrtpchannel::pointer other )
{
  projectrtpchannel::pointer tmpother;
  if( this == other.get() )
  {
    return true;
  }

  projectchannelmux::pointer m = this->others.load( std::memory_order_relaxed );
  if( nullptr == m )
  {
    m = projectchannelmux::create( this->iocontext );
    m->addchannel( shared_from_this() );
    m->addchannel( other );
    /* We don't need our channel timer */
    m->go();
  }
  else
  {
    m->addchannel( other );
  }

  return true;
}

/*!md
## unmix
As it says.
*/
void projectrtpchannel::unmix( void )
{
std::cout << "unmix" << std::endl << std::flush;
  this->others = nullptr;
}

/*!md
## audio
The CODECs on the other end which are acceptable. The first one should be the preferred. For now we keep hold of the list of codecs as we may be using them in the future. Filter out non-RTP streams (such as DTMF).
*/
bool projectrtpchannel::audio( codeclist codecs )
{
  this->codecs = codecs;
  codeclist::iterator it;
  for( it = codecs.begin(); it != codecs.end(); it++ )
  {
    switch( *it )
    {
      case PCMAPAYLOADTYPE:
      case PCMUPAYLOADTYPE:
      case G722PAYLOADTYPE:
      case ILBCPAYLOADTYPE:
      {
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
            boost::asio::ip::udp::resolver::iterator it )
{
  boost::asio::ip::udp::resolver::iterator end;

  if( it == end )
  {
    /* Failure - silent (the call will be as well!) */
    return;
  }

  this->confirmedrtpsenderendpoint = *it;
  this->targetconfirmed = true;
}

/*!md
## handlesend
What is called once we have sent something.
*/
void projectrtpchannel::handlesend(
      const boost::system::error_code& error,
      std::size_t bytes_transferred)
{

}

/*!md
## handlertcpdata
We have received some RTCP data - now do something with it.
*/
void projectrtpchannel::handlertcpdata( void )
{

}
