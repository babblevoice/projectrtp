
#include <boost/bind/bind.hpp>
#include <boost/chrono.hpp>

#include "projectrtpchannelmux.h"

/*
## c'stor && create
*/
projectchannelmux::projectchannelmux( boost::asio::io_context &iocontext ):
  channels(),
  iocontext( iocontext ),
  tick( iocontext ),
  nexttick( std::chrono::high_resolution_clock::now() ),
  newchannels(),
  newchannelslock( false ),
  added(),
  subtracted() {
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

    rtppacket *src;
    while( true ) {
      AQUIRESPINLOCK( chan->rtpbufferlock );
      src = chan->inbuff->peek();
      RELEASESPINLOCK( chan->rtpbufferlock );

      AQUIRESPINLOCK( chan->rtpdtlslock );
      dtlssession::pointer currentdtlssession = chan->rtpdtls;
      RELEASESPINLOCK( chan->rtpdtlslock );
      if( nullptr != currentdtlssession &&
          !currentdtlssession->rtpdtlshandshakeing ) {
        if( !currentdtlssession->unprotect( src ) ) {
          chan->receivedpkskip++;
          src = nullptr;
        }
      }

      if( !chan->checkfordtmf( src ) ) break;

      for( auto& dtmfchan: this->channels ) {
        if( dtmfchan->recv && chan.get() != dtmfchan.get() ) {
          this->postrtpdata( chan, dtmfchan, src );
        }
      }

      AQUIRESPINLOCK( chan->rtpbufferlock );
      chan->inbuff->poppeeked();
      RELEASESPINLOCK( chan->rtpbufferlock );
    }

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

    if( chan->recv ) {
      AQUIRESPINLOCK( chan->rtpbufferlock );
      rtppacket *src = chan->inbuff->peek();
      RELEASESPINLOCK( chan->rtpbufferlock );
      if( nullptr != src ) {
        this->subtracted -= chan->incodec;
      }
      AQUIRESPINLOCK( chan->rtpbufferlock );
      chan->inbuff->poppeeked();
      RELEASESPINLOCK( chan->rtpbufferlock );
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
With 2 channels DTMF is also passed through.
*/
void projectchannelmux::mix2( void ) {
  auto chans = this->channels.begin();
  auto chan1 = *chans++;
  auto chan2 = *chans;
  rtppacket *src;

  while( true ) {
    AQUIRESPINLOCK( chan1->rtpbufferlock );
    src = chan1->inbuff->pop();
    RELEASESPINLOCK( chan1->rtpbufferlock );

    AQUIRESPINLOCK( chan1->rtpdtlslock );
    dtlssession::pointer currentdtlssession = chan1->rtpdtls;
    RELEASESPINLOCK( chan1->rtpdtlslock );
    if( nullptr != currentdtlssession &&
        !currentdtlssession->rtpdtlshandshakeing ) {
      if( !currentdtlssession->unprotect( src ) ) {
        chan1->receivedpkskip++;
        src = nullptr;
      }
    }

    if( !chan1->checkfordtmf( src ) ) break;
    this->postrtpdata( chan1, chan2, src );
  }
  this->postrtpdata( chan1, chan2, src );

  while( true ) {
    AQUIRESPINLOCK( chan2->rtpbufferlock );
    src = chan2->inbuff->pop();
    RELEASESPINLOCK( chan2->rtpbufferlock );

    AQUIRESPINLOCK( chan2->rtpdtlslock );
    dtlssession::pointer currentdtlssession = chan2->rtpdtls;
    RELEASESPINLOCK( chan2->rtpdtlslock );
    if( nullptr != currentdtlssession &&
        !currentdtlssession->rtpdtlshandshakeing ) {
      if( !currentdtlssession->unprotect( src ) ) {
        chan2->receivedpkskip++;
        src = nullptr;
      }
    }

    if( !chan2->checkfordtmf( src ) ) break;
    this->postrtpdata( chan2, chan1, src );
  }
  this->postrtpdata( chan2, chan1, src );
}

/*
Our timer handler.
*/
void projectchannelmux::handletick( const boost::system::error_code& error ) {
  if ( error == boost::asio::error::operation_aborted ) return;

  this->checkfornewmixes();

  /* Check for channels which have request removal */
  for ( projectchanptrlist::iterator chan = this->channels.begin();
        chan != this->channels.end(); ) {
    if( ( *chan )->removemixer ) {
      ( *chan )->mixing = false;
      ( *chan )->removemixer = false;
      ( *chan )->mixer = nullptr;
      chan = this->channels.erase( chan );
    } else {
      ++chan;
    }
  }

  /* if 1 is removed then we may have 1 left - and 1 isn't mixing */
  if( 1 == this->channels.size() ) {
    projectchanptrlist::iterator chan = this->channels.begin();
    ( *chan )->mixing = false;
    ( *chan )->removemixer = false;
    ( *chan )->mixer = nullptr;
    chan = this->channels.erase( chan );
  }

  if( 0 == this->channels.size() ) {
    return;
  }

  for( auto& chan: this->channels ) {
    chan->startticktimer();
  }

  for( auto& chan: this->channels ) {
    chan->checkfornewrecorders();
  }

  for( auto& chan: this->channels ) {
    chan->dtlsnegotiate();
    chan->incrtsout();
  }

  if( 2 == this->channels.size() ) {
    this->mix2();
  } else if( this->channels.size() > 2 ) {
    this->mixall();
  }

  for( auto& chan: this->channels ) {
    chan->senddtmf();
    chan->writerecordings();
    chan->checkidlerecv();
  }

  for( auto& chan: this->channels ) {
    chan->endticktimer();
  }

  /* The last thing we do */
  this->setnexttick();
}

void projectchannelmux::setnexttick( void ) {
  this->nexttick = this->nexttick + std::chrono::milliseconds( 20 );

  this->tick.expires_after( this->nexttick - std::chrono::high_resolution_clock::now() );
  this->tick.async_wait( boost::bind( &projectchannelmux::handletick,
                                      shared_from_this(),
                                      boost::asio::placeholders::error ) );
}

void projectchannelmux::go( void ) {
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

void projectchannelmux::addchannels( projectrtpchannelptr chana, projectrtpchannelptr chanb ) {
  AQUIRESPINLOCK( this->newchannelslock );
  this->newchannels.push_back( chana );
  this->newchannels.push_back( chanb );
  RELEASESPINLOCK( this->newchannelslock );
}

/*
## postrtpdata
Send the data somewhere.
*/
void projectchannelmux::postrtpdata( projectrtpchannelptr srcchan, projectrtpchannelptr dstchan, rtppacket *src ) {
  if( nullptr == src ) return;
  rtppacket *dst = dstchan->gettempoutbuf();

  if( nullptr == dst ) {
    fprintf( stderr, "We have a null out buffer\n" );
    return;
  }

  if( src->getpayloadtype() == srcchan->rfc2833pt ) {
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
