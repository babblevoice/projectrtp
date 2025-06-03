#ifndef PROJECTRTPREADSTREAM_H
#define PROJECTRTPREADSTREAM_H

#include <node_api.h>
#include <vector>
#include <string>

class projectrtpchannel;

class projectrtpreadstream {
public:
  projectrtpreadstream( napi_env env, projectrtpchannel* channel, const std::string& encoding, bool combined );
  ~projectrtpreadstream();

  void emitdata( const std::vector<uint8_t>& data );

  napi_value init_js_object();
  void set_data_callback( napi_ref cb );
  void set_end_callback( napi_ref cb );

  bool combined;

private:
  napi_env env_;
  napi_ref js_this_;
  napi_ref on_data_cb_;
  napi_ref on_end_cb_;
  napi_threadsafe_function tsfn_;

  std::string encoding_;

  projectrtpchannel* channel_;
};

#endif
