
#ifndef PROJECTRTPBUFFER_H
#define PROJECTRTPBUFFER_H


#include <memory>
#include <vector>
#include <queue>

#include "projectrtppacket.h"

/*
My thoughts on buffers. We reorder as we might want to use the data
and will will probably hepl the other end if we are proxying.

If the buffer is too big we can introduce delay. If it is too short
then we might introduce losses.
*/

class rtpbuffer :
  public std::enable_shared_from_this< rtpbuffer > {

public:

  rtpbuffer( int count, int waterlevel );
  ~rtpbuffer();

  rtpbuffer( const rtpbuffer& ) = delete;              // copy ctor
  rtpbuffer( rtpbuffer&& ) = delete;                   // move ctor
  rtpbuffer& operator=( const rtpbuffer& ) = delete;   // copy assignment
  rtpbuffer& operator=( rtpbuffer&& ) = delete;        // move assignment

  typedef std::shared_ptr< rtpbuffer > pointer;
  static pointer create( int count, /* size of the array to store packets */
                         int waterlevel /* the level we build up before allowing a read */ );

  rtppacket* peek( void );
  rtppacket* pop( void );
  void push( void );
  rtppacket* reserve( void );
  uint64_t getdropped( void ) { return this->dropped; }

  typedef std::vector< rtppacket > rtppackets;
  typedef std::vector< rtppacket* > rtppacketptrs;
  typedef std::queue< rtppacket* > qrtppacketptrs;

private:

  /*
    buffer - used as the actual buffer for data
    availablertpdata - all of the items in buffer which are free to use
    orderedrtpdata - we have received data and verified it ok - so it needs
                     processing - items are in order based on sn
    reserved - after we reserve - this is where it is held until filled
  */
  rtppackets buffer;
  qrtppacketptrs availablertpdata;
  rtppacketptrs orderedrtpdata;
  rtppacket *reserved;

  int buffercount;
  int waterlevel;
  uint16_t outsn;
  uint64_t dropped;
};

#ifdef NODE_MODULE

#include <node_api.h>
void initrtpbuffer( napi_env env, napi_value &result );

#endif /* NODE_MODULE */


#endif /* PROJECTRTPBUFFER_H */
