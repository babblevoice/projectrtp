

#ifndef PROJECTRTPCODECX_H
#define PROJECTRTPCODECX_H


#include <ilbc.h>
#include <spandsp.h>

#include "projectrtppacket.h"
#include "projectrtppacket.h"
#include "globals.h"
#include "firfilter.h"


/*
Helper class for sound to manipulate sound - before or after a rtppacket is required.
*/
class codecx;
class rawsound
{
public:
  rawsound();
  rawsound( uint8_t *ptr, std::size_t samples, int format, uint16_t samplerate );
  rawsound( rtppacket& pk, bool dirty = false );
  rawsound( rawsound & );
  ~rawsound();

  uint8_t *c_str( void ){ return this->data; };
  size_t size( void ){ return this->samples; };
  void size( size_t samplecount ){ this->samples = samplecount; };
  inline int getformat( void ){ return this->format; }; /* aka payload type */
  uint16_t getsamplerate( void ) { return this->samplerate; };
  size_t getbytespersample( void ) { return this->bytespersample; };
  void malloc( size_t samplecount, size_t bytespersample, int format );
  void zero( void );

  /* needed for << operato on codecx */
  inline int getpayloadtype( void ) { return this->format; }
  inline void setpayloadlength( size_t length ) { this->samples = length; };
  inline void setlength( size_t length ) { this->samples = length; };
  inline bool isdirty( void ) { return this->dirtydata; }
  inline void dirty( bool d = true ) { this->dirtydata = d; }
  void copy( uint8_t *src, size_t len );
  void copy( rawsound &other );

  rawsound& operator+=( codecx& rhs );
  rawsound& operator-=( codecx& rhs );

private:
  void frompt( int payloadtype );

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

  /* mark the fact that whilst we may have space, we may not have any actual data */
  bool dirtydata;
};

class codecx
{
  friend class rawsound;
public:
  codecx();
  ~codecx();

  void reset( void );
  void restart( void );
  uint16_t power( void );

  bool hasdata() { return this->_hasdata; }
  rawsound& getref( int pt );

  friend codecx& operator << ( codecx&, rawsound& );
  friend codecx& operator << ( codecx&, rtppacket& );

  friend auto* operator << ( auto*, codecx& );
  friend auto& operator << ( auto&, codecx& );

  friend codecx& operator << ( codecx&, const char& );
  static const char next;

private:
  bool alaw2ulaw( void );
  bool ulaw2alaw( void );
  bool g711tol16( void );
  bool ilbctol16( void );
  bool g722tol16( void );
  bool l16topcma( void );
  bool l16topcmu( void );
  bool l16tog722( void );
  bool l16toilbc( void );

  bool l16lowtowideband( void);
  bool l16widetonarrowband( void );
  bool requirenarrowband( void );
  bool requirewideband( void );
  rawsound& requirel16( void );

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

  bool _hasdata;
};

/* Functions */
void gen711convertdata( void );
void codectests( void );


/* Template functions */
/*
## rtppacket << codecx
Take the data out and transcode if necessary. Keep reference to any packet we have transcoded as we may need to use it again.

pk needs to impliment:
int getpayloadtype()
copy( *, size_t )
setpayloadlength( size_t )
setlength( size_t )
*/
auto& operator << ( auto& pk, codecx& c )
{
  (&pk) << c;
  return pk;
}

auto* operator << ( auto *pk, codecx& c )
{
  int outpayloadtype = pk->getpayloadtype();

  switch( outpayloadtype )
  {
    case ILBCPAYLOADTYPE:
    {
      c.ilbcref = rawsound( *pk, true );
      break;
    }
    case G722PAYLOADTYPE:
    {
      c.g722ref = rawsound( *pk, true );
      break;
    }
    case PCMAPAYLOADTYPE:
    {
      c.pcmaref = rawsound( *pk, true );
      break;
    }
    case PCMUPAYLOADTYPE:
    {
      c.pcmuref = rawsound( *pk, true );
      break;
    }
    case L168KPAYLOADTYPE:
    {
      c.l168kref = rawsound( *pk, true );
      break;
    }

    case L1616KPAYLOADTYPE:
    {
      c.l1616kref = rawsound( *pk, true );
      break;
    }
  }

  c.getref( outpayloadtype );
  return pk;
}


#endif /* PROJECTRTPCODECX_H */
