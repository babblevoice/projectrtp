
#include <iostream>

#include "projectrtppacket.h"
#include "globals.h"


/*!md
## Constructor
*/
rtppacket::rtppacket() :
  length( 0 )
{
  memset( this->pk, 0, RTPMAXLENGTH );
}

/*!md
## Copy contructor

We only need to copy the required bytes.
*/
rtppacket::rtppacket( rtppacket &p )
{
  this->length = p.length;
  memcpy( this->pk, p.pk, p.length );
}

/*!md
## init
Sets up all of the RTP packet header to defaults.
*/
void rtppacket::init( uint32_t ssrc )
{
  memset( this->pk, 0, 12 );
  this->pk[ 0 ] = 0x80; /* v = 2 */

  uint32_t *ssrcptr = ( uint32_t * )this->pk;
  ssrcptr += 2;
  *ssrcptr = htonl( ssrc );
}

/*!md
## Copy
Copy the payload from src to us.
*/
void rtppacket::copy( rtppacket *src )
{
  if( !src ) return;

  memcpy( this->pk + 12 + ( this->getpacketcsrccount() * 4 ),
          src->pk + 12 + ( src->getpacketcsrccount() * 4 ),
          src->getpayloadlength() );

  this->length = src->getpayloadlength() + 12 + ( this->getpacketcsrccount() * 4 );
}

/*!md
## Copy
Copy the payload from src to us.
*/
void rtppacket::copy( uint8_t *src, size_t len )
{
  if( !src ) return;

  memcpy( this->pk + 12 + ( this->getpacketcsrccount() * 4 ),
          src,
          len );

  this->length = len + 12 + ( this->getpacketcsrccount() * 4 );
}

/*!md
## Copy header
*/
void rtppacket::copyheader( rtppacket *src )
{
  if( !src ) return;
  this->length = src->length;
  memcpy( this->pk, src->pk, 12 + ( src->getpacketcsrccount() * 4 ) );
}




/*!md
## getpacketversion
As it says.
*/
uint8_t rtppacket::getpacketversion( void )
{
  return ( this->pk[ 0 ] & 0xc0 ) >> 6;
}

/*!md
## getpacketpadding
As it says.
*/
uint8_t rtppacket::getpacketpadding( void )
{
  return ( this->pk[ 0 ] & 0x20 ) >> 5;
}

/*!md
## getpacketextension
As it says.
*/
uint8_t rtppacket::getpacketextension( void )
{
  return ( this->pk[ 0 ] & 0x10 ) >> 4;
}

/*!md
## getpacketcsrccount
As it says.
*/
uint8_t rtppacket::getpacketcsrccount( void )
{
  return ( this->pk[ 0 ] & 0x0f );
}

/*!md
## getpacketmarker
As it says.
*/
uint8_t rtppacket::getpacketmarker( void )
{
  return ( this->pk[ 1 ] & 0x80 ) >> 7;
}

/*!md
## getpayloadtype
As it says.
*/
uint8_t rtppacket::getpayloadtype( void )
{
  return ( this->pk[ 1 ] & 0x7f );
}

/*!md
## setpayloadtype
As it says. We also set the length of the packet to a default amount for that CODEC.
*/
void rtppacket::setpayloadtype( uint8_t pt )
{
  this->pk[ 1 ] = ( this->pk[ 1 ] & 0x80 ) | ( pt & 0x7f );

  switch( pt )
  {
    case ILBC20PAYLOADBYTES:
    {
      this->length = 12 + ILBC20PAYLOADBYTES;
      break;
    }
    case ILBC30PAYLOADBYTES:
    {
      this->length = 12 + ILBC30PAYLOADBYTES;
      break;
    }
    default:
    {
      this->length = 12 + G711PAYLOADBYTES;
    }
  }
}

/*!md
## getsequencenumber
As it says.
*/
uint16_t rtppacket::getsequencenumber( void )
{
  uint16_t *tmp = ( uint16_t * )this->pk;
  tmp++;
  return ntohs( *tmp );
}

/*!md
## getsequencenumber
As it says.
*/
void rtppacket::setsequencenumber( uint16_t sq )
{
  uint16_t *tmp = ( uint16_t * )this->pk;
  tmp++;
  *tmp = htons( sq );
}

/*!md
## gettimestamp
As it says.
*/
uint32_t rtppacket::gettimestamp( void )
{
  uint32_t *tmp = ( uint32_t * )this->pk;
  tmp++;
  return ntohl( *tmp );
}

/*!md
## getnexttimestamp
Takes the current timesamp and adds the number of samples (depending on the CODEC) and returns that value - so it can be assigned to the next packet.
*/
uint32_t rtppacket::getnexttimestamp( void )
{
  return this->gettimestamp() + this->getticksperpacket();
}

/*!md
## getticksperpacket
Returns the number of ticks per packet. All support packets are the same. A bit strange!
*/
uint32_t rtppacket::getticksperpacket( void )
{
  return G711PAYLOADBYTES;
}

/*!md
## settimestamp
As it says.
*/
void rtppacket::settimestamp( uint32_t tmstp )
{
  uint32_t *tmp = ( uint32_t * )this->pk;
  tmp++;
  *tmp = htonl( tmstp );
}

/*!md
## getssrc
As it says.
*/
uint32_t rtppacket::getssrc( void )
{
  uint32_t *tmp = ( uint32_t * )this->pk;
  tmp += 2;
  return ntohl( *tmp );
}

/*!md
## getcsrc
As it says. Use getpacketcsrccount to return the number of available
0-15. This function doesn't check bounds.
*/
uint32_t rtppacket::getcsrc( uint8_t index )
{
  uint32_t *tmp = ( uint32_t * )this->pk;
  tmp += 3 + index;
  return ntohl( *tmp );
}

/*!md
## getpayload
Returns a pointer to the start of the payload.
*/
uint8_t *rtppacket::getpayload( void )
{
  uint8_t *ptr = this->pk;
  ptr += 12;
  ptr += ( this->getpacketcsrccount() * 4 );
  return ptr;
}


/*!md
## getpayloadlength
As it says.
*/
uint16_t rtppacket::getpayloadlength( void )
{

  if( this->length < ( 12 - ( (size_t)this->getpacketcsrccount() * 4 ) ) )
  {
    std::cerr << "RTP Packet has a nonsense size" << std::endl;
    return 0;
  }
  return this->length - 12 - ( this->getpacketcsrccount() * 4 );
}

/*!md
## setpayloadlength
As it says.
*/
void rtppacket::setpayloadlength( size_t length )
{
  this->length = 12 + ( this->getpacketcsrccount() * 4 ) + length;
}
