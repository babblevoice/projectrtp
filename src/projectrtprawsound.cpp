
#include <iostream>   // cerr
#include <cstdlib>
#include <iomanip>

#include "projectrtprawsound.h"
#include "projectrtpcodecx.h"

/*!md
# rawsound
An object representing raw data for sound - which can be in any format (supported). We maintain a pointer to the raw data and will not clean it up.
*/
rawsound::rawsound() :
  data( nullptr ),
  samples( 0 ),
  allocatedlength( 0 ),
  bytespersample( 1 ),
  format( 0 ),
  samplerate( 0 ),
  dirtydata( false )
{
}

/*!md
# rawsound
*/
rawsound::rawsound( uint8_t *ptr, std::size_t samples, int format, uint16_t samplerate ) :
  data( ptr ),
  samples( samples ),
  allocatedlength( 0 ),
  bytespersample( 1 ),
  format( format ),
  samplerate( samplerate ),
  dirtydata( false )
{
  this->frompt( format );
}

rawsound::rawsound( const rawsound &o ) :
  data( o.data ),
  samples( o.samples ),
  allocatedlength( 0 ),
  bytespersample( o.bytespersample ),
  format( o.format ),
  samplerate( o.samplerate ),
  dirtydata( o.dirtydata ) {
}

rawsound& rawsound::operator=( const rawsound& o ) {

  if( this != &o ) { // not a self-assignment
    this->data = o.data;
    this->samples = o.samples;
    this->allocatedlength = 0;
    this->bytespersample = o.bytespersample;
    this->format = o.format;
    this->samplerate = o.samplerate;
    this->dirtydata = o.dirtydata;
  }

  return *this;
}

/*!md
## frompt
From payload Type. Configure samplerate and bytes per sample etc.
*/
void rawsound::frompt( int payloadtype )
{
  switch( payloadtype )
  {
    default:
    case PCMUPAYLOADTYPE:
    case PCMAPAYLOADTYPE:
    {
      this->samplerate = 8000;
      this->bytespersample = 1;
      break;
    }
    case G722PAYLOADTYPE:
    {
      this->samplerate = 16000;
      this->bytespersample = 1;
      break;
    }
    case ILBCPAYLOADTYPE:
    {
      this->samplerate = 8000;
      this->bytespersample = 1;
      break;
    }
    /* The next 2 can only come from a sound file */
    case L168KPAYLOADTYPE:
    {
      this->samplerate = 8000;
      if( 1 == this->bytespersample )
      {
        this->bytespersample = 2;
        this->samples = this->samples / 2;
      }
      break;
    }
    case L1616KPAYLOADTYPE:
    {
      this->samplerate = 16000;
      if( 1 == this->bytespersample )
      {
        this->bytespersample = 2;
        this->samples = this->samples / 2;
      }
      break;
    }
  }
}

/*!md
## rawsound
Construct from an rtp packet
*/
rawsound::rawsound( rtppacket& pk, bool dirty ) :
  data( pk.getpayload() ),
  samples( pk.getpayloadlength() ),
  allocatedlength( 0 ),
  bytespersample( 1 ),
  format( pk.getpayloadtype() ),
  samplerate( 8000 ),
  dirtydata( dirty )
{
  this->frompt( this->format );
}

/*!md
## d-tor
Tidy up.
*/
rawsound::~rawsound()
{
  if( this->allocatedlength > 0 && nullptr != this->data )
  {
    delete[] this->data;
    this->data = nullptr;
  }
  this->allocatedlength = 0;
}

/*
## zero
Reset buffer with zero
*/
void rawsound::zero( void ) {
  if( nullptr == this->data ) return;

  size_t zeroamount = this->samples * this->bytespersample;

  if( 0 != this->allocatedlength && zeroamount > this->allocatedlength ) {
    std::cerr << "Trying to zero memory but capped by allocated amount" << std::endl;
    zeroamount = this->allocatedlength;
  }

  memset( this->data, 0, zeroamount );
}

/*
## copy
* Target (this) MUST have memory available.
* format must be set appropriatly before the call
*/
void rawsound::copy( uint8_t *src, size_t len ) {
  if( nullptr == this->data ) return;

  memcpy( this->data,
          src,
          len );
}

/*
## copy (from other)
* Target (this) MUST have data allocated.
*/
void rawsound::copy( rawsound &other ) {

  if( nullptr == this->data || this->samples < other.samples ) return;

  this->bytespersample = other.bytespersample;
  this->samplerate = other.samplerate;
  this->format = other.format;
  this->samples = other.samples;

  memcpy( this->data,
          other.data,
          other.samples * other.bytespersample );

}

/*
## Add and subtract

Used for mixing in channels.

void rawsound::add( void )
{

}

void rawsound::subtract( void )
{

}
*/

/*
## malloc
Allocate our own memory.
*/
void rawsound::malloc( size_t samplecount, size_t bytespersample, int format ) {
  this->samples = samplecount;
  this->bytespersample = bytespersample;
  this->format = format;
  size_t requiredsize = samplecount * bytespersample;

  if( this->allocatedlength > 0 ) {
    if( this->allocatedlength >= requiredsize ) {
      return;
    }

    delete[] this->data;
  }

  /* make sure data is aligned as ARM and others can throw SIGSEGV on misalignment */
  switch( bytespersample ) {
    case 1:
    {
      this->data = new uint8_t[ samplecount ];
      break;
    }
    case 2:
    {
      this->data = ( uint8_t * ) new uint16_t[ samplecount ];
      break;
    }
    case 4:
    {
      this->data = ( uint8_t * ) new uint32_t[ samplecount ];
      break;
    }
    default:
    {
      /* this shouldn't happen - in fact - please avoid */
      this->data = new uint8_t[ requiredsize ];
    }
  }
  
  this->allocatedlength = requiredsize;
}

/*
## operator +=
Used for mixing audio
*/
rawsound& rawsound::operator+=( codecx& rhs ) {
  size_t length;

  int16_t *in;
  int16_t *out = ( int16_t * ) this->data;

  switch( this->format ) {
    case L168KPAYLOADTYPE: {
      rhs.requirenarrowband();
      if( rhs.l168kref.isdirty() ) return *this;
      length = rhs.l168kref.size();
      in = ( int16_t * ) rhs.l168kref.c_str();
      break;
    }
    case L1616KPAYLOADTYPE: {
      rhs.requirewideband();
      if( rhs.l1616kref.isdirty() ) return *this;
      length = rhs.l1616kref.size();
      in = ( int16_t * ) rhs.l1616kref.c_str();
      break;
    }
    default: {
      std::cerr << "Attemping to perform an addition on a none linear format" << std::endl;
      return *this;
    }
  }

  if( length > this->samples ) {
    std::cerr << "We have been asked to add samples but we don't have enough space" << std::endl;
    length = this->samples;
  }

  for( size_t i = 0; i < length; i++ ) {
    *out++ += *in++;
  }

  return *this;
}

rawsound& rawsound::operator-=( codecx& rhs )
{
  size_t length;

  int16_t *in;
  int16_t *out = ( int16_t * ) this->data;

  switch( this->format )
  {
    case L168KPAYLOADTYPE:
    {
      rhs.requirenarrowband();

      length = rhs.l168kref.size();
      in = ( int16_t * ) rhs.l168kref.c_str();
      break;
    }
    case L1616KPAYLOADTYPE:
    {
      rhs.requirewideband();

      length = rhs.l1616kref.size();
      in = ( int16_t * ) rhs.l1616kref.c_str();
      break;
    }
    default:
    {
      std::cerr << "Attemping to perform a subtract on a none linear format" << std::endl;
      return *this;
    }
  }

  if( length > this->samples )
  {
    std::cerr << "We have been asked to subtract samples but we don't have enough space" << std::endl;
    length = this->samples;
  }

  for( size_t i = 0; i < length; i++ )
  {
    *out++ -= *in++;
  }

  return *this;
}


void rawsound::dump() {

  std::cout << "=================BEGIN=================" << std::endl;
  std::cout << "samples: " << std::dec << this->samples << std::endl;
  std::cout << "bytespersample: " << std::dec << this->bytespersample << std::endl;
  std::cout << "format: " << std::dec << this->format << std::endl;
  
  auto v = this->data;
  size_t len = this->samples * this->bytespersample;
  for( size_t i = 0; i < len; i ++ ) {
    std::cout << std::showbase << std::setfill( '0' ) << std::setw( 2 ) << std::hex << std::right << +(*(v+i)) << ' ';
    if( 0 != i && 0 == i % 16 ) std::cout << std::endl;
  }
  std::cout << std::dec;
  std::cout << std::endl << "=================END===================" << std::endl;
}