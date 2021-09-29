
#include <boost/bind/bind.hpp>
#include <boost/chrono.hpp>

#include "projectrtpchannelmux.h"

/*
## c'stor && create
*/
projectchannelmux::projectchannelmux( boost::asio::io_context &iocontext ):
  iocontext( iocontext ),
  tick( iocontext ),
  newchannelslock( false ),
  mixing( false ) {
}

projectchannelmux::~projectchannelmux() {
  this->tick.cancel();
}

projectchannelmux::pointer projectchannelmux::create( boost::asio::io_context &iocontext ) {
  return pointer( new projectchannelmux( iocontext ) );
}

void projectchannelmux::mixall( void ) {
  /* First decide on a common rate (if we only have 8K channels it is pointless
  upsampling them all and wasting resources) */
  int l16krequired = L168KPAYLOADTYPE;
  size_t insize = L16PAYLOADSAMPLES;

  for( auto& chan: this->channels ) {
    switch( chan->codec ) {
      case G722PAYLOADTYPE:
      case L1616KPAYLOADTYPE: {
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
  for( auto& chan: this->channels ) {
    if( !chan->recv ) continue;
    
    AQUIRESPINLOCK( chan->rtpbufferlock );
    rtppacket *src = chan->inbuff->peek();
    RELEASESPINLOCK( chan->rtpbufferlock );

    if( nullptr != src ) {
      chan->incodec << codecx::next;
      chan->incodec << *src;
      this->added += chan->incodec;
    }
  }

  /* Now we subtract this channel to send to this channel. */
  for( auto& chan: this->channels ) {
    if( !chan->send ) continue;

    rtppacket *dst = chan->gettempoutbuf();

    this->subtracted.zero();
    this->subtracted.copy( this->added );

    /*
     There is a small chance that rtp bottom may have flipped from nullptr to something.
     We will get a little noise as a result. We could get rid of this by marking the
     channel somehow?
    */
    if( chan->recv ) {
      AQUIRESPINLOCK( chan->rtpbufferlock );
      rtppacket *src = chan->inbuff->pop();
      RELEASESPINLOCK( chan->rtpbufferlock );
      if( nullptr != src ) {
        this->subtracted -= chan->incodec;
      }
    }

    chan->outcodec << codecx::next;
    chan->outcodec << this->subtracted;
    dst << chan->outcodec;
    chan->writepacket( dst );
  }
}

/*
## mix2
More effient mixer for 2 channels
The caller has to ensure there are 2 channels.
*/
void projectchannelmux::mix2( void ) {
  auto chans = this->channels.begin();
  auto chan1 = *chans++;
  auto chan2 = *chans;
  rtppacket *src;

  AQUIRESPINLOCK( chan1->rtpbufferlock );
  src = chan1->inbuff->pop();
  RELEASESPINLOCK( chan1->rtpbufferlock );

  if( src != nullptr ) {
    this->checkfordtmf( chan1, src );
    this->postrtpdata( chan1, chan2, src );
  }

  AQUIRESPINLOCK( chan2->rtpbufferlock );
  src = chan2->inbuff->pop();
  RELEASESPINLOCK( chan2->rtpbufferlock );

  if( src != nullptr ) {
    this->checkfordtmf( chan2, src );
    this->postrtpdata( chan2, chan1, src );
  }
}

/*
Our timer handler.
*/
void projectchannelmux::handletick( const boost::system::error_code& error ) {
  if ( error != boost::asio::error::operation_aborted )
  {
    boost::posix_time::ptime nowtime( boost::posix_time::microsec_clock::local_time() );

    this->checkfornewmixes();

    for( auto& chan: this->channels ) {
      chan->checkfornewrecorders();
    }

    /* Check for channels which have request removal */
    this->channels.remove_if( []( projectrtpchannelptr chan ) { return !chan->mixing; } );

    for( auto& chan: this->channels ) {
      chan->incrtsout();
    }

    if( 2 == this->channels.size() ) {
      this->mix2();
    } else if( this->channels.size() > 2 ) {
      this->mixall();
    }

    for( auto& chan: this->channels ) {
      chan->writerecordings();
      chan->checkidlerecv();
    }

    /* calc our timer */
    boost::posix_time::time_duration const diff = ( boost::posix_time::microsec_clock::local_time() - nowtime );
    uint64_t tms = diff.total_microseconds();
    for( auto& chan: this->channels ) {
      chan->totalticktime += tms;
      chan->totaltickcount++;
      if( tms > chan->maxticktime ) chan->maxticktime = tms;
    }


    /* The last thing we do */
    if( this->channels.size() > 0 ) {
      this->nexttick = this->nexttick + std::chrono::milliseconds( 20 );

      this->tick.expires_after( this->nexttick - std::chrono::high_resolution_clock::now() );
      this->tick.async_wait( boost::bind( &projectchannelmux::handletick,
                                          shared_from_this(),
                                          boost::asio::placeholders::error ) );
    }
  }
}

void projectchannelmux::go( void ) {

  if( this->mixing.exchange( true ) ) return;

  this->nexttick = std::chrono::high_resolution_clock::now() + std::chrono::milliseconds( 20 );

  this->tick.expires_after( this->nexttick - std::chrono::high_resolution_clock::now() );
  this->tick.async_wait( boost::bind( &projectchannelmux::handletick,
                                        shared_from_this(),
                                        boost::asio::placeholders::error ) );
}

/*
Check for new channels to add to the mix in our own thread.
*/
void projectchannelmux::checkfornewmixes( void ) {

  AQUIRESPINLOCK( this->newchannelslock );

  for ( auto const& newchan : this->newchannels ) {

    /* Don't add duplicates */
    for( auto& checkchan: this->channels ) {
      if( checkchan.get() == newchan.get() ) goto contin;
    }

    this->channels.push_back( newchan );
    contin:;
  }
  this->newchannels.clear();

  RELEASESPINLOCK( this->newchannelslock );
}

void projectchannelmux::addchannel( projectrtpchannelptr chan ) {
  AQUIRESPINLOCK( this->newchannelslock );
  this->newchannels.push_back( chan );
  RELEASESPINLOCK( this->newchannelslock );
}

/*
## checkfordtmf
*/
void projectchannelmux::checkfordtmf( projectrtpchannelptr chan, rtppacket *src ) {
  /* The next section is sending to our recipient(s) */
  if( 0 != chan->rfc2833pt && src->getpayloadtype() == chan->rfc2833pt ) {
    /* We have to look for DTMF events handling issues like missing events - such as the marker or end bit */
    uint16_t sn = src->getsequencenumber();
    uint8_t event = 0;
    uint8_t endbit = 0;

    /*
    there really should be a packet - we should cater for multiple?
    endbits can appear to be sent multiple times.
    */
    if( src->getpayloadlength() >= 4 ) {
      uint8_t * pl = src->getpayload();
      endbit = pl[ 1 ] >> 7;
      event = pl[ 0 ];
    }

    uint8_t pm = src->getpacketmarker();
    if( !pm && 0 != chan->lasttelephoneevent && abs( static_cast< long long int >( sn - chan->lasttelephoneevent ) ) > 20 ) {
      pm = 1;
    }

    if( pm ) {
#warning finish me - how to we signal back DTMF
#if 0
      if( chan->control ) {
        JSON::Object v;
        v[ "action" ] = "telephone-event";
        v[ "id" ] = chan->id;
        v[ "uuid" ] = chan->uuid;
        v[ "event" ] = ( JSON::Integer )event;

        chan->control->sendmessage( v );
      }
#endif
    }

    if( endbit ) {
      chan->lasttelephoneevent = 0;
    } else {
      chan->lasttelephoneevent = sn;
    }
  }
}

/*
## postrtpdata
Send the data somewhere.
*/
void projectchannelmux::postrtpdata( projectrtpchannelptr srcchan, projectrtpchannelptr dstchan, rtppacket *src ) {
  rtppacket *dst = dstchan->gettempoutbuf();

  if( nullptr == dst ) {
    std::cerr << "We have a null out buffer" << std::endl;
    return;
  }

  /* This needs testing */
  if( 0 != srcchan->rfc2833pt && src->getpayloadtype() == srcchan->rfc2833pt ) {
    dst->setpayloadtype( srcchan->rfc2833pt );
    dst->copy( src );
  } else {
    srcchan->incodec << codecx::next;
    srcchan->incodec << *src;

    dstchan->outcodec << codecx::next;
    dstchan->outcodec << *src;
    dst << dstchan->outcodec;
  }

  dstchan->writepacket( dst );
}
