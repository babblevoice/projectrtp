

#ifndef PROJECTRTPCODECX_H
#define PROJECTRTPCODECX_H


#include <ilbc.h>
#include <spandsp.h>

#include "projectrtppacket.h"
#include "projectrtppacket.h"
#include "globals.h"
#include "firfilter.h"

class rawsound
{
public:
  rawsound();
  rawsound( uint8_t *ptr, std::size_t length, int format, uint16_t samplerate = 8000 );
  rawsound( rtppacket& pk );
  rawsound( rawsound & );
  ~rawsound();

  uint8_t *c_str( void ){ return this->data; };
  size_t size( void ){ return this->length; };
  void size( size_t len ){ this->length = len; };
  int getformat( void ){ return this->format; };
  uint16_t getsamplerate( void ) { return this->samplerate; };
  void malloc( size_t len );

private:

  /* ptr to our buffer */
  uint8_t *data;

  /* length of buffer in bytes */
  size_t length;
  size_t allocatedlength;
  
  int format;
  uint16_t samplerate;
};

class codecx
{
public:
  codecx();
  ~codecx();

  void reset( void );
  void restart( void );

  friend codecx& operator << ( codecx&, rtppacket& );
  friend rtppacket& operator << ( rtppacket&, codecx& );
  friend codecx& operator << ( codecx&, rawsound& );
  friend rawsound& operator << ( rawsound&, codecx& );
  friend codecx& operator << ( codecx&, const char& );
  static const char next;

private:
  void xlaw2ylaw( void );
  bool g711tol16( void );
  bool ilbctol16( void );
  bool g722tol16( void );
  void l16topcma( void );
  void l16topcmu( void );
  void l16tog722( void );
  void l16toilbc( void );

  void l16lowtowideband( void);
  void l16widetonarrowband( void );
  void requirenarrowband( void );
  void requirewideband( void );

  /* CODECs  */
  g722_encode_state_t *g722encoder;
  g722_decode_state_t *g722decoder;

  iLBC_encinst_t *ilbcencoder;
  iLBC_decinst_t *ilbcdecoder;

  /* If we require downsampling */
  lowpass3_4k16k lpfilter;
  /* When we up sample we need to interpolate so need last sample */
  int16_t resamplelastsample;
#if 0
  int16_t *l168k; /* narrow band */
  int16_t *l1616k; /* wideband */

  uint16_t l168kallocatedlength;
  uint16_t l1616kallocatedlength;
  int16_t l168klength;
  int16_t l1616klength;

  rawsound in;
#endif
  rawsound l168kref;
  rawsound l1616kref;
  rawsound pcmaref;
  rawsound pcmuref;
  rawsound g722ref;
  rawsound ilbcref;

};


codecx& operator << ( codecx&, rtppacket& );
rtppacket& operator << ( rtppacket&, codecx& );

codecx& operator << ( codecx&, rawsound& );
rawsound& operator << ( rawsound&, codecx& );

codecx& operator << ( codecx&, const char& );

/* Functions */
void gen711convertdata( void );


#endif /* PROJECTRTPCODECX_H */
