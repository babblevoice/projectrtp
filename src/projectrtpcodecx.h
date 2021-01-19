

#ifndef PROJECTRTPCODECX_H
#define PROJECTRTPCODECX_H


#include <ilbc.h>
#include <spandsp.h>

#include "projectrtppacket.h"
#include "projectrtppacket.h"
#include "globals.h"
#include "firfilter.h"


/*
 This class is not tracking enough information. We store the number of bytes - but not
 what, always what is stored in the object - so we lose the sample rate. If we have length of 160
 we don't know if this is 160 sample or 320 samples. We do keep track of samplerate which is
 helpful but converting l1616K to l168k gets a bit tricky
*/
class rawsound
{
public:
  rawsound();
  rawsound( uint8_t *ptr, std::size_t length, int format, uint16_t samplerate = 8000 );
  rawsound( rtppacket& pk );
  rawsound( rawsound & );
  ~rawsound();

  uint8_t *c_str( void ){ return this->data; };
  size_t size( void ){ return this->samples; };
  void size( size_t samplecount ){ this->samples = samplecount; };
  int getformat( void ){ return this->format; };
  uint16_t getsamplerate( void ) { return this->samplerate; };
  void malloc( size_t samplecount, size_t bytespersample, int format );

private:

  /* ptr to our buffer */
  uint8_t *data;

  /* sample count of buffer in bytes */
  size_t samples;

  /* The amount we requested from the system malloc */
  size_t allocatedlength;

  size_t bytespersample;

  /* see globals.h */
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
  //friend void codectests( void );
  static const char next;

private:
  void alaw2ulaw( void );
  void ulaw2alaw( void );
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
void codectests( void );


#endif /* PROJECTRTPCODECX_H */
