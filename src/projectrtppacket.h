

#ifndef PROJECTRTPPACKTET_H
#define PROJECTRTPPACKTET_H


/* CODECs */
#include <ilbc.h>
#include <spandsp.h>

#include "firfilter.h"
#include "globals.h"



class rtppacket
{
public:
  rtppacket();
  rtppacket( rtppacket & );
  size_t length;
  uint8_t pk[ RTPMAXLENGTH ];

  uint8_t getpacketversion( void );
  uint8_t getpacketpadding( void );
  uint8_t getpacketextension( void );
  uint8_t getpacketcsrccount( void );
  uint8_t getpacketmarker( void );
  uint8_t getpayloadtype( void );
  uint16_t getsequencenumber( void );
  uint32_t gettimestamp( void );
  uint32_t getnexttimestamp( void );
  uint32_t getticksperpacket( void );
  uint32_t getssrc( void );
  uint32_t getcsrc( uint8_t index );
  uint8_t *getpayload( void );
  uint16_t getpayloadlength( void );

  void setpayloadlength( size_t length );
  void setpayloadtype( uint8_t payload );
  void setsequencenumber( uint16_t sq );
  void settimestamp( uint32_t tmstp );
  void init( uint32_t ssrc );

  void copy( rtppacket *src );
  void copy( uint8_t *src, size_t len );
  void copyheader( rtppacket *src );

};


#endif  /* PROJECTRTPPACKTET_H */
