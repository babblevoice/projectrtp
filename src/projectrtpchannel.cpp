

#include <iostream>

#include <boost/bind/bind.hpp>
#include <boost/chrono.hpp>
#include <iomanip>


#include "projectrtpchannel.h"
#include "controlclient.h"

using namespace boost::placeholders;

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
  orderedinminsn( 0 ),
  orderedinmaxsn( 0 ),
  orderedinbottom( 0 ),
  lastworkedonsn( 0 ),
  rtpindexoldest( 0 ),
  rtpindexin( 0 ),
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
  doecho( false ),
  mixqueue( MIXQUEUESIZE ),
  tick( iocontext ),
  tickswithnortpcount( 0 )
{
  memset( this->orderedrtpdata, 0, sizeof( this->orderedrtpdata ) );
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

  /* indexes into our circular rtp array */
  this->rtpindexin = 0;
  this->rtpindexoldest = 0;


  this->receivedpkcount = 0;
  this->receivedpkskip = 0;

  this->rtpoutindex = 0;

  this->codecworker.reset();

  this->rtpsocket.open( boost::asio::ip::udp::v4() );
  this->rtpsocket.bind( boost::asio::ip::udp::endpoint(
      boost::asio::ip::udp::v4(), this->port ) );

  this->rtcpsocket.open( boost::asio::ip::udp::v4() );
  this->rtcpsocket.bind( boost::asio::ip::udp::endpoint(
      boost::asio::ip::udp::v4(), this->port + 1 ) );

  this->receivedrtp = false;
  this->active = true;

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

  for( int i = 0; i < BUFFERDELAYCOUNT; i++ )
  {
    this->orderedrtpdata[ i ] = nullptr;
  }
  this->orderedinminsn = 0;
  this->orderedinmaxsn = 0;
  this->orderedinbottom = 0;
  this->lastworkedonsn = 0;

  this->tickswithnortpcount = 0;

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

  /* remove oursevelse from our list of mixers */
  if( this->others )
  {
    projectrtpchannellist::iterator it;
    for( it = this->others->begin(); it != this->others->end(); it++ )
    {
      if( it->get() == this )
      {
        this->others->erase( it );
        break;
      }
    }
    /* release the shared pointer */
    this->others = nullptr;
  }

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

      v[ "stats" ] = s;
    }

    this->control->sendmessage( v );
  }
}

/*!md
## handletick
Our timer to send data
*/
void projectrtpchannel::handletick( const boost::system::error_code& error )
{
  if ( error != boost::asio::error::operation_aborted )
  {
    this->tickswithnortpcount++;
    if( this->tickswithnortpcount > 400 )
    {
      this->close();
    }

    this->checkfornewmixes();

    /* only us */
    if( !this->others || 0 == this->others->size() )
    {
      stringptr newplaydef = std::atomic_exchange( &this->newplaydef, stringptr( NULL ) );
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
      }
      else if( this->player )
      {
        rtppacket *out = this->gettempoutbuf();
        rawsound r = player->read();
        if( 0 != r.size() )
        {
          this->codecworker << codecx::next;
          this->codecworker << r;
          *out << this->codecworker;
          this->writepacket( out );
        }
      }
    }

    while( this->handlertpdata() );


    /* The last thing we do */
    this->tick.expires_at( this->tick.expiry() + boost::asio::chrono::milliseconds( 20 ) );
    this->tick.async_wait( boost::bind( &projectrtpchannel::handletick,
                                        shared_from_this(),
                                        boost::asio::placeholders::error ) );
  }
}

/*!md
## handlereadsomertp
Wait for RTP data. We have to re-order when required. Look after all of the round robin memory here. We should have enough time to deal with the in data before it gets overwritten.
*/
void projectrtpchannel::readsomertp( void )
{
  this->rtpsocket.async_receive_from(
    boost::asio::buffer( &this->rtpdata[ this->rtpindexin ].pk, RTPMAXLENGTH ),
                          this->rtpsenderendpoint,
      [ this ]( boost::system::error_code ec, std::size_t bytes_recvd )
      {
        if ( !ec && bytes_recvd > 0 && bytes_recvd <= RTPMAXLENGTH )
        {
#ifdef SIMULATEDPACKETLOSSRATE
          /* simulate packet loss */
          if( 0 == rand() % SIMULATEDPACKETLOSSRATE )
          {
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
            this->confirmedrtpsenderendpoint = this->rtpsenderendpoint;
            this->receivedrtp = true;
            this->lastworkedonsn = this->rtpdata[ this->rtpindexin ].getsequencenumber() - 1;
          }

          /* After the first packet - we only accept data from the verified source */
          if( this->confirmedrtpsenderendpoint != this->rtpsenderendpoint )
          {
            return;
          }

          this->rtpdata[ this->rtpindexin ].length = bytes_recvd;

          /* Now order it */
          rtppacket *src = &this->rtpdata[ this->rtpindexin ];
          uint16_t sn = src->getsequencenumber();

          this->orderedrtpdata[ sn % BUFFERPACKETCOUNT ] = src;

          if( sn > this->orderedinmaxsn ) this->orderedinmaxsn = sn;

          /* Indicate where we start */
          if( sn > ( this->orderedinminsn + BUFFERPACKETCOUNT ) )
          {
            this->orderedinminsn = this->orderedinmaxsn = sn;
            this->orderedinbottom = sn % BUFFERPACKETCOUNT;
          }
        }

        if( !ec && bytes_recvd && this->active )
        {
          this->rtpindexin = ( this->rtpindexin + 1 ) % BUFFERPACKETCOUNT;
          this->readsomertp();
        }
      } );
}


/*!md
## handlertpdata

Buffer up RTP data to reorder and give time for packets to be received then process.

Return false to indicate complete - no further work to do, return true to indicate there may be more work to do.
*/
bool projectrtpchannel::handlertpdata( void )
{
  if( !this->receivedrtp ) return false;

  rtppacket *src = this->orderedrtpdata[ this->orderedinbottom ];
  if( nullptr == src ) return false;

  uint16_t sn = src->getsequencenumber();
  uint16_t aheadby = this->orderedinmaxsn - sn;

  /* We allow BUFFERDELAYCOUNT to accumulate in our buffer before we work on them */
  if( aheadby < BUFFERDELAYCOUNT ) return false;

  uint16_t workingonaheadby = sn - this->lastworkedonsn;

  /* Only process if it is the expected sn */
  if( this->orderedinminsn == sn )
  {
    this->processrtpdata( src, workingonaheadby - 1 );
  }

  this->lastworkedonsn = sn;
  this->orderedrtpdata[ this->orderedinbottom ] = nullptr;
  this->orderedinminsn++;
  this->orderedinbottom = ( this->orderedinbottom + 1 ) % BUFFERPACKETCOUNT;

  return true;
}


/*!md
## processrtpdata

Mix and send the data somewhere.
*/
void projectrtpchannel::processrtpdata( rtppacket *src, uint32_t skipcount )
{
  this->receivedpkskip += skipcount;

  /* The next section is sending to our recipient(s) */
  if( 0 != this->rfc2833pt && src->getpayloadtype() == this->rfc2833pt )
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
    if( !pm && 0 != this->lasttelephoneevent && abs( static_cast< long long int >( sn - this->lasttelephoneevent ) ) > 20 )
    {
      pm = 1;
    }

    if( pm )
    {
      if( this->control )
      {
        JSON::Object v;
        v[ "action" ] = "telephone-event";
        v[ "id" ] = this->id;
        v[ "uuid" ] = this->uuid;
        v[ "event" ] = ( JSON::Integer )event;

        this->control->sendmessage( v );
      }
    }

    if( endbit )
    {
      this->lasttelephoneevent = 0;
    }
    else
    {
      this->lasttelephoneevent = sn;
    }
  }

  if( this->others && 2 == this->others->size() )
  {
    /* one should be us */
    projectrtpchannellist::iterator it = this->others->begin();
    projectrtpchannel::pointer chan = *it;
    if( it->get() == this )
    {
      chan = *( ++it );
    }

    rtppacket *dst = chan->gettempoutbuf( skipcount );

    /* This needs testing */
    if( 0 != this->rfc2833pt && src->getpayloadtype() == this->rfc2833pt )
    {
      dst->setpayloadtype( this->rfc2833pt );
      dst->copy( src );
    }
    else
    {
      this->codecworker << codecx::next;
      this->codecworker << *src;
      *dst << this->codecworker;
    }

    chan->writepacket( dst );
  }
  else if( this->doecho && ( !this->others || 1 == this->others->size() ) )
  {
    this->codecworker << codecx::next;
    this->codecworker << *src;
    rtppacket *dst = this->gettempoutbuf( skipcount );
    *dst << this->codecworker;
    this->writepacket( dst );
  }

  return;
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
  /* Create our others list */
  if( !this->others )
  {
    this->others = projectrtpchannellistptr( new projectrtpchannellist  );
  }

  this->mixqueue.push( other );

  return true;
}

/*!md
## unmix
As it says.
*/
void projectrtpchannel::unmix( void )
{
  this->mix( projectrtpchannel::pointer() );
}

/*!md
## checkfornewmixes
This is the mechanism how we can use multiple threads and not screw u our data structures - without using mutexes.
*/
void projectrtpchannel::checkfornewmixes( void )
{
  projectrtpchannel::pointer other;

  while( this->mixqueue.pop( other ) )
  {
    if( !other )
    {
      /* empty indicates unmix */

      /* Allow us to remix with another */
      this->receivedrtp = false;

      /* Clear others */
      for( auto it = this->others->begin(); it != this->others->end(); it++ )
      {
        ( *it )->unmix();
      }
      this->others->clear();
      return;
    }

    /* ensure no duplicates */
    bool usfound = false;
    bool themfound = false;
    for( auto it = this->others->begin(); it != this->others->end(); it++ )
    {
      if( it->get() == this )
      {
        usfound = true;
      }

      if( *it == other )
      {
        themfound = true;
      }
    }

    if( !usfound )
    {
      this->others->push_back( other );
    }

    if( !themfound )
    {
      this->others->push_back( shared_from_this() );
    }

    other->others = this->others;
  }
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
  /* Don't override the symmetric port we send back to */
  if( this->receivedrtp )
  {
    return;
  }

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
