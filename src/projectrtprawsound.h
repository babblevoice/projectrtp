

#ifndef PROJECTRTPRAWSOUND_H
#define PROJECTRTPRAWSOUND_H

#include "projectrtppacket.h"

class codecx;
class rawsound {
public:
  rawsound();
  rawsound( uint8_t *ptr, std::size_t samples, int format, uint16_t samplerate );
  rawsound( rtppacket& pk, bool dirty = false );
  rawsound( const rawsound &o );

  rawsound& operator=( const rawsound& rhs );

  ~rawsound();

  uint8_t *c_str( void ){ return this->data; };
  size_t size( void ){ return this->samples; };
  void size( size_t samplecount ){ this->samples = samplecount; };
  inline int getformat( void ){ return this->format; }; /* aka payload type */
  uint16_t getsamplerate( void ) { return this->samplerate; };
  size_t getbytespersample( void ) { return this->bytespersample; };
  void malloc( size_t samplecount, size_t bytespersample, int format );
  void zero( void );

  bool hasdata() { return this->size() != 0 && !this->isdirty(); }

  /* needed for << operato on codecx */
  inline int getpayloadtype( void ) { return this->format; }
  inline void setpayloadlength( size_t length ) { this->samples = length; };
  inline void setlength( size_t length ) { this->samples = length; };
  inline bool isdirty( void ) { return this->dirtydata; }
  inline void dirty( bool d = true ) { this->dirtydata = d; if( d && 0 == allocatedlength ) data = nullptr; }
  void copy( uint8_t *src, size_t len );
  void copy( rawsound &other );

  rawsound& operator+=( codecx& rhs );
  rawsound& operator-=( codecx& rhs );

  void dump();

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

#endif /* PROJECTRTPRAWSOUND_H */
