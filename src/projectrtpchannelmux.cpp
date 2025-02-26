
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
  channelslock( false ),
  added(),
  subtracted(),
  active( false ) {
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
  int l16format = L168KPAYLOADTYPE;
  size_t insize = L16PAYLOADSAMPLES;

  for( auto& chan: this->channels ) {
    switch( chan->codec ) {
      case G722PAYLOADTYPE:
      case L1616KPAYLOADTYPE: {
        l16format = L1616KPAYLOADTYPE;
        insize = L1616PAYLOADSAMPLES;
        goto endofforloop;
      }
    }
  }
  endofforloop:

  /* allocate the max needed */
  this->added.malloc( insize, sizeof( int16_t ), l16format );
  this->subtracted.malloc( insize, sizeof( int16_t ), l16format );
  this->added.zero();

  /* We first have to add them all up */
  for( auto& chan: this->channels ) {
    if( !chan->recv ) continue;

    rtppacket *src;
    while( true ) {
      {
        SpinLockGuard guard( chan->rtpbufferlock );
        src = chan->inbuff->peek();
      }

      if( nullptr == src ) break;

      dtlssession::pointer currentdtlssession;
      {
        SpinLockGuard guard( chan->rtpdtlslock );
        currentdtlssession = chan->rtpdtls;
      }

      if( nullptr != currentdtlssession &&
          !currentdtlssession->rtpdtlshandshakeing ) {
        if( !currentdtlssession->unprotect( src ) ) {
          chan->receivedpkskip++;
          src = nullptr;

          {
            SpinLockGuard guard( chan->rtpbufferlock );
            chan->inbuff->poppeeked();
          }

          break;
        }
      }

      if( !chan->checkfordtmf( src ) ) break;

      for( auto& dtmfchan: this->channels ) {
        if( dtmfchan->recv && chan.get() != dtmfchan.get() ) {
          this->postrtpdata( chan, dtmfchan, src );
        }
      }
      /* remove the DTMF packet */
      {
        SpinLockGuard guard( chan->rtpbufferlock );
        chan->inbuff->poppeeked();
      }
    }

    if( nullptr != src ) {
      chan->incodec << *src;
      this->added += chan->incodec;
    }
  }

  /* Now we subtract this channel to send to this channel. */
  for( auto& chan: this->channels ) {
    if( !chan->send ) {
      SpinLockGuard guard( chan->rtpbufferlock );
      chan->inbuff->poppeeked();
      continue;
    }

    rtppacket *dst = chan->gettempoutbuf();

    /* start with a direct copy */
    this->subtracted.copy( this->added );

    if( chan->recv ) {
      rtppacket *src;
      {
        SpinLockGuard guard( chan->rtpbufferlock );
        src = chan->inbuff->peeked();
      }

      if( nullptr != src ) {
        this->subtracted -= chan->incodec;
      }

      {
        SpinLockGuard guard( chan->rtpbufferlock );
        chan->inbuff->poppeeked();
      }
    }

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
    {
      SpinLockGuard guard( chan1->rtpbufferlock );
      src = chan1->inbuff->pop();
    }

    if( nullptr == src ) break;

    dtlssession::pointer currentdtlssession;
    {
      SpinLockGuard guard( chan1->rtpdtlslock );
      currentdtlssession = chan1->rtpdtls;
    }


    if( nullptr != currentdtlssession &&
        !currentdtlssession->rtpdtlshandshakeing ) {
      if( !currentdtlssession->unprotect( src ) ) {
        chan1->receivedpkskip++;
        src = nullptr;
        break;
      }
    }

    if( !chan1->checkfordtmf( src ) ) break;
    this->postrtpdata( chan1, chan2, src );
  }
  this->postrtpdata( chan1, chan2, src );

  while( true ) {
    {
      SpinLockGuard guard( chan2->rtpbufferlock );
      src = chan2->inbuff->pop();
    }

    if( nullptr == src ) break;


    dtlssession::pointer currentdtlssession;    
    {
      SpinLockGuard guard( chan2->rtpdtlslock );
      currentdtlssession = chan2->rtpdtls;
    }

    if( nullptr != currentdtlssession &&
        !currentdtlssession->rtpdtlshandshakeing ) {
      if( !currentdtlssession->unprotect( src ) ) {
        chan2->receivedpkskip++;
        src = nullptr;
        break;
      }
    }

    if( !chan2->checkfordtmf( src ) ) break;
    this->postrtpdata( chan2, chan1, src );
  }
  this->postrtpdata( chan2, chan1, src );
}

bool projectchannelmux::channelremoverequested( const projectrtpchannelptr& chan ) {
  if( chan->removemixer || chan->_requestclose || (!chan->active) ) {
    chan->dounmix();
    return true;
  }

  return false;
}

/*
Our timer handler.
*/
void projectchannelmux::handletick( const boost::system::error_code& error ) {
  if ( error == boost::asio::error::operation_aborted ) return;

  /* ensure we are not destroyed  during tick */
  auto self = shared_from_this();

  if( !this->active ) return;

  projectchanptrlist workingchannels;
  {
    SpinLockGuard guard( this->channelslock );
    /* Check for channels which have request removal */
    this->channels.remove_if( channelremoverequested );
    /* ensure we have a local copy to maintain life of all objects */
    workingchannels = this->channels;
  }

  if( 0 == workingchannels.size() ) {
    /* We're done */
    this->active = false;
    return;
  }

  for( auto& chan: workingchannels ) {
    chan->outcodec << codecx::next;
    chan->incodec << codecx::next;

    {
      SpinLockGuard guard( chan->playerlock );
      /* preserve any players during mix */
      chan->playerstash = chan->player;
    }
    
    chan->startticktimer();
    chan->incrtsout();
  }

  if( 2 == workingchannels.size() ) {
    this->mix2();
  } else if( workingchannels.size() > 2 ) {
    this->mixall();
  }

  for( auto& chan: workingchannels ) {
    chan->senddtmf();
    chan->writerecordings();  //////////////////// crash here -> channel -> soundfile - there is a problem with the CODEC - this can remove recorders from channel
    chan->checkidlerecv();    // this can call doclose() - recorders might be destroyed now
    chan->endticktimer();

    chan->playerstash = nullptr;
  }

  /* The last thing we do */
  this->setnexttick( self );
}

void projectchannelmux::setnexttick( pointer self ) {
  this->nexttick = this->nexttick + std::chrono::milliseconds( 20 );

  this->tick.expires_after( this->nexttick - std::chrono::high_resolution_clock::now() );
  this->tick.async_wait( boost::bind( &projectchannelmux::handletick,
                                      self,
                                      boost::asio::placeholders::error ) );
}

void projectchannelmux::go( void ) {

  if( this->active ) return;
  this->active = true;

  this->nexttick = std::chrono::high_resolution_clock::now() + std::chrono::milliseconds( 20 );

  this->tick.expires_after( this->nexttick - std::chrono::high_resolution_clock::now() );
  this->tick.async_wait( boost::bind( &projectchannelmux::handletick,
                                        shared_from_this(),
                                        boost::asio::placeholders::error ) );
}

/**
 * @brief Compare left and right for underlying pointer equality.
 * Used for list.unique()
 * 
 * @param l 
 * @param r 
 * @return true 
 * @return false 
 */
static bool underlyingpointerequal( projectrtpchannelptr l, projectrtpchannelptr r ){
  return l.get() == r.get();
}

/**
 * @brief Used for sort - compare the underlying pointer so we can then use unique to remove
 * duplicates.
 * 
 * @param l 
 * @param r 
 * @return true 
 * @return false 
 */
static bool underlyingpointercmp( projectrtpchannelptr l, projectrtpchannelptr r ){
  return l.get() > r.get();
}

void projectchannelmux::addchannel( projectrtpchannelptr chan ) {
  SpinLockGuard guard( this->channelslock );

  chan->mixing = true;
  this->channels.push_back( chan );

  this->channels.sort( underlyingpointercmp );
  this->channels.unique( underlyingpointerequal );
}

void projectchannelmux::addchannels( projectrtpchannelptr chana, projectrtpchannelptr chanb ) {
  SpinLockGuard guard( this->channelslock );

  chana->mixing = true;
  chanb->mixing = true;
  this->channels.push_back( chana );
  this->channels.push_back( chanb );

  this->channels.sort( underlyingpointercmp );
  this->channels.unique( underlyingpointerequal );
}

/*
## postrtpdata
Send the data somewhere.
*/
void projectchannelmux::postrtpdata( projectrtpchannelptr srcchan, projectrtpchannelptr dstchan, rtppacket *src ) {
  if( nullptr == src ) return;
  rtppacket *dst = dstchan->gettempoutbuf();

  if( nullptr == dst ) {
    dstchan->outpkdropcount++;
    fprintf( stderr, "We have a null out buffer\n" );
    return;
  }

  if( src->getpayloadtype() == RFC2833PAYLOADTYPE ) {
    dst->setpayloadtype( dstchan->rfc2833pt );
    dst->copy( src );
  } else {
    srcchan->incodec << *src;
    dstchan->outcodec << *src;
    dst << dstchan->outcodec;
  }

  dstchan->writepacket( dst );
}
